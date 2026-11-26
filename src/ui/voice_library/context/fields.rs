use super::{text, weak};
use crate::app::picker::{Picker, Selection};
use crate::synthesis::runtime::{ContextImportDialog, ContextImportPhase, ContextImportSource};
use crate::tr;
use eframe::egui::*;

pub(super) fn show(
    ui: &mut Ui,
    dialog: &mut ContextImportDialog,
    picker: &mut Option<Picker>,
) -> bool {
    match dialog.source {
        ContextImportSource::SessionFile => {
            *picker = session_file(ui, dialog);
            false
        }
        ContextImportSource::File => {
            *picker = file(ui, dialog);
            false
        }
        ContextImportSource::Manual => manual(ui, dialog),
        ContextImportSource::Online => online(ui, dialog),
    }
}

fn session_file(ui: &mut Ui, dialog: &ContextImportDialog) -> Option<Picker> {
    text(
        ui,
        tr!("选择本机 Synthesizer V Studio 2 的 session 登录会话文件，无需启动原编辑器。"),
    );
    weak(
        ui,
        tr!("点击“导入”后才会读取，并将此声库的上下文保存到 RV 的 context.json。"),
    );
    if let Some(path) = &dialog.session_path {
        ui.add(Label::new(path.display().to_string()).wrap());
    } else {
        weak(ui, tr!("尚未选择会话文件"));
    }
    ui.button(tr!("选择会话文件…")).clicked().then(|| {
        Picker::new(Selection::File, tr!("选择 Synthesizer V Studio 2 的会话文件"))
            .starting_at(dialog.session_path.as_ref().and_then(|path| path.parent()))
    })
}

fn file(ui: &mut Ui, dialog: &ContextImportDialog) -> Option<Picker> {
    text(ui, tr!("选择包含此声库及对应模型代际的上下文 JSON 文件。"));
    if let Some(path) = &dialog.file_path {
        ui.add(Label::new(path.display().to_string()).wrap());
    } else {
        weak(ui, tr!("尚未选择文件"));
    }
    ui.button(tr!("选择上下文文件…")).clicked().then(|| {
        Picker::new(Selection::File, tr!("选择模型上下文文件"))
            .filter(tr!("模型上下文 JSON"), &["json"])
            .starting_at(dialog.file_path.as_ref().and_then(|path| path.parent()))
    })
}

fn manual(ui: &mut Ui, dialog: &mut ContextImportDialog) -> bool {
    text(ui, tr!("填写此声库及对应模型代际的 identity 模型条件值。"));
    ui.add(
        TextEdit::singleline(&mut dialog.manual_identity)
            .desired_width(f32::INFINITY)
            .hint_text(tr!("十进制整数或 0x 十六进制"))
            .password(true),
    )
    .changed()
}

fn online(ui: &mut Ui, dialog: &mut ContextImportDialog) -> bool {
    text(
        ui,
        tr!("打开并登录 Synthesizer V Studio 2，选择其进程后导入。完成后可关闭原编辑器。"),
    );
    if dialog.phase == ContextImportPhase::Discovering {
        ui.horizontal(|ui| {
            ui.spinner();
            text(ui, tr!("正在查找已打开的编辑器…"));
        });
        return false;
    }
    if dialog.sources.is_empty() {
        weak(ui, tr!("未找到可用的编辑器进程。打开原编辑器后点击刷新。"));
        return false;
    }
    let mut changed = false;
    ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
        for source in &dialog.sources {
            changed |= ui
                .radio_value(
                    &mut dialog.selected_process,
                    Some(source.process_id),
                    format!("{} · PID {}", source.label, source.process_id),
                )
                .changed();
        }
    });
    changed
}
