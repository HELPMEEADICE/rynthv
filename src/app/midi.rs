use super::App;
use super::picker::{Picker, Selection};
use crate::{editor::Editor, midi::MidiFile, model::beats, tr, trf, ui};
use eframe::egui::Context;
use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MidiImportMode {
    NewProject,
    Tracks,
}

pub enum MidiImportPhase {
    Loading(Receiver<Result<MidiFile, String>>),
    Ready { file: MidiFile, selected: Vec<bool> },
    Failed(String),
}

pub struct MidiImport {
    pub path: PathBuf,
    pub mode: MidiImportMode,
    pub phase: MidiImportPhase,
    pub validation_error: Option<String>,
}

impl App {
    pub fn begin_midi_import(&mut self, mode: MidiImportMode, ctx: &Context) {
        ui::phonemes::phoneme_deactivate(self);
        ui::piano::cancel_audition(self);
        ui::tempo_input::finish(self);
        self.commit_lyric();
        self.editor.finish();
        self.pick_one(
            ctx,
            Picker::new(Selection::File, tr!("导入 MIDI"))
                .filter(tr!("MIDI 文件"), &["mid", "midi"]),
            move |app, ctx, path| app.load_midi_import(path, mode, ctx),
        );
    }

    pub(super) fn load_midi_import(&mut self, path: PathBuf, mode: MidiImportMode, ctx: &Context) {
        #[cfg(not(target_arch = "wasm32"))]
        let phase = {
            use std::sync::mpsc;
            let (tx, rx) = mpsc::channel();
            let source = path.clone();
            let repaint = ctx.clone();
            let worker = std::thread::Builder::new()
                .name("midi-import".into())
                .spawn(move || {
                    let _ = tx.send(crate::midi::load(&source));
                    repaint.request_repaint();
                });
            match worker {
                Ok(_) => MidiImportPhase::Loading(rx),
                Err(error) => MidiImportPhase::Failed(trf!("无法启动 MIDI 读取线程：{}", error)),
            }
        };
        #[cfg(target_arch = "wasm32")]
        let phase = {
            use crate::{
                platform::web::{client, protocol::Request},
                synthesis::notify::Notify,
            };
            let repaint = ctx.clone();
            MidiImportPhase::Loading(
                client::submit(
                    Request::Midi { path: path.clone() },
                    Notify::new(move || repaint.request_repaint()),
                    |_, _| {},
                )
                .into_receiver(),
            )
        };
        self.midi_import = Some(MidiImport {
            path,
            mode,
            phase,
            validation_error: None,
        });
    }

    pub(super) fn poll_midi_import(&mut self) {
        let Some(import) = &mut self.midi_import else {
            return;
        };
        let MidiImportPhase::Loading(receiver) = &import.phase else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(tr!("MIDI 读取线程意外停止。").into()),
        };
        import.phase = match result {
            Ok(file) => {
                let selected = file
                    .tracks
                    .iter()
                    .map(|track| !track.notes.is_empty())
                    .collect();
                MidiImportPhase::Ready { file, selected }
            }
            Err(error) => MidiImportPhase::Failed(error),
        };
    }

    pub fn apply_midi_import(&mut self, import: &MidiImport) -> Result<(), String> {
        ui::phonemes::phoneme_deactivate(self);
        let MidiImportPhase::Ready { file, selected } = &import.phase else {
            return Err(tr!("MIDI 尚未读取完成。").into());
        };
        let indices: Vec<_> = selected
            .iter()
            .enumerate()
            .filter_map(|(index, &selected)| selected.then_some(index))
            .collect();
        self.commit_lyric();
        self.editor.finish();
        let summary: crate::editor::ImportSummary = match import.mode {
            MidiImportMode::NewProject => {
                // Construct and validate before retiring the current project.
                let (editor, summary) = Editor::from_midi(file, &indices)?;
                self.synthesis.reset_project();
                #[cfg(target_os = "android")]
                {
                    self.recovery.invalidate();
                    self.recovered_dirty = true;
                }
                self.editor = editor;
                self.path = None;
                self.audio_import = None;
                self.gesture = None;
                self.piano_anchor = crate::ui::piano::AnchorSelection::default();
                self.parameter_lane = ui::lanes::ParameterSelection::default();
                self.phoneme_editing = ui::phonemes::PhonemeEditing::default();
                self.group_gesture = None;
                self.audio_drag = None;
                self.mixer_drag = None;
                self.view_pan = ui::view_pan::ViewPan::default();
                self.rename = None;
                self.time_edit = None;
                self.bpm_edit = None;
                self.lyric = None;
                self.phoneme_draft = ui::panels::PhonemeDraft::default();
                self.fill_lyrics = None;
                summary
            }
            MidiImportMode::Tracks => self.editor.import_midi_tracks(file, &indices)?,
        };
        self.piano_collapsed = false;
        self.view.arrangement_collapsed = false;
        if let Some(first) = self.editor.notes().first() {
            let offset = self
                .editor
                .active_ref()
                .map_or(0, |reference| reference.blick_offset);
            let start = beats(first.onset + offset).floor().max(0.0);
            self.view.piano.start = start;
            self.view.arrangement.start = start;
            self.view.top_pitch = (first.pitch + 9).clamp(12, 127) as f32;
        }
        self.status = trf!(
            "已导入 {} 个音符到 {} 条音轨",
            summary.notes,
            summary.tracks
        );
        if summary.split_tracks > 0 {
            self.status
                .push_str(&trf!(" · 重叠音符拆出 {} 条声部", summary.split_tracks));
        }
        Ok(())
    }
}
