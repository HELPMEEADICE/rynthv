use super::{string_method, with_activity};
use eframe::egui::{self, Context, Event, IMEPurpose, ImeEvent, Key, RawInput};
use serde::Deserialize;
use std::{
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};

static READY: AtomicBool = AtomicBool::new(false);

thread_local! {
    static KEYBOARD: Cell<i32> = const { Cell::new(0) };
    static INSETS: Cell<[i32; 4]> = const { Cell::new([0; 4]) };
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Input {
    Insets {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    },
    Preedit {
        text: String,
    },
    Commit {
        text: String,
    },
    Delete {
        before: usize,
        after: usize,
    },
    Text {
        text: String,
    },
    Key {
        text: String,
    },
}

pub(super) fn ready() {
    READY.store(true, Ordering::Release);
}

pub(crate) fn read(ctx: &Context, raw: &mut RawInput) {
    if READY.swap(false, Ordering::AcqRel)
        && let Ok(Some(events)) = string_method(jni::jni_str!("rynthvTakeInput"))
        && let Ok(events) = serde_json::from_str::<Vec<Input>>(&events)
    {
        for input in events {
            match input {
                Input::Insets {
                    left,
                    top,
                    right,
                    bottom,
                } => INSETS.set([left, top, right, bottom]),
                Input::Preedit { text } => raw.events.push(Event::Ime(ImeEvent::Preedit {
                    text,
                    active_range_chars: None,
                })),
                Input::Commit { text } => raw.events.push(Event::Ime(ImeEvent::Commit(text))),
                Input::Delete { before, after } => {
                    raw.events.push(Event::Ime(ImeEvent::DeleteSurrounding {
                        before_chars: before,
                        after_chars: after,
                    }))
                }
                Input::Text { text } => raw.events.push(Event::Text(text)),
                Input::Key { text } => {
                    if let Some(key) = Key::from_name(&text) {
                        for pressed in [true, false] {
                            raw.events.push(Event::Key {
                                key,
                                physical_key: None,
                                pressed,
                                repeat: false,
                                modifiers: egui::Modifiers::NONE,
                            });
                        }
                    }
                }
            }
        }
    }
    let scale = raw.viewport().native_pixels_per_point.unwrap_or(1.0) * ctx.zoom_factor();
    let [left, top, right, bottom] = INSETS.get().map(|px| px as f32 / scale);
    raw.safe_area_insets = Some(egui::SafeAreaInsets(egui::epaint::MarginF32 {
        left,
        top,
        right,
        bottom,
    }));
}

pub(crate) fn publish(ctx: &Context) {
    let (ime, copied) = ctx.output_mut(|output| {
        let copied = output
            .commands
            .iter()
            .rev()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            });
        // NativeActivity's default InputQueue only forwards key events.
        (output.ime.take(), copied)
    });
    let purpose = match ime.map(|ime| ime.purpose) {
        None => 0,
        Some(IMEPurpose::Normal) => 1,
        Some(IMEPurpose::Password) => 2,
        Some(IMEPurpose::Terminal) => 3,
    };
    let reset = ime.is_some_and(|ime| ime.should_interrupt_composition);
    if KEYBOARD.replace(purpose) != purpose || reset {
        let _ = with_activity(|env, activity| {
            env.call_method(
                activity,
                jni::jni_str!("rynthvUpdateKeyboard"),
                jni::jni_sig!("(IZ)V"),
                &[purpose.into(), reset.into()],
            )?;
            Ok(())
        });
    }
    if let Some(text) = copied {
        let _ = with_activity(|env, activity| {
            let text = env.new_string(&text)?;
            env.call_method(
                activity,
                jni::jni_str!("rynthvCopyText"),
                jni::jni_sig!("(Ljava/lang/String;)V"),
                &[(&text).into()],
            )?;
            Ok(())
        });
    }
}

pub(crate) fn clipboard() -> Result<Option<String>, String> {
    string_method(jni::jni_str!("rynthvReadClipboard"))
}
