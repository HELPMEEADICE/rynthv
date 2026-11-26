//! Validate mixer and export features before publishing or writing audio.

use crate::model::Extra;
use serde_json::Value;
use crate::{tr, trf};

pub(super) fn validate_project_mixer(value: Option<&Value>) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    let object = value.as_object().ok_or(tr!("工程 projectMixer 必须为对象"))?;
    for (key, value) in object {
        match key.as_str() {
            "linkRoomSettings" if value.is_boolean() => {}
            _ => return Err(trf!("暂不支持工程混音设置 projectMixer.{} = {}", key, value)),
        }
    }
    Ok(())
}

pub(super) fn validate_mixer(mixer: &Extra) -> Result<(), String> {
    for (key, value) in mixer {
        let supported = match key.as_str() {
            "gainDecibel" => value
                .as_f64()
                .is_some_and(|db| (10.0_f64.powf(db / 20.0) as f32).is_finite()),
            "pan" => value
                .as_f64()
                .is_some_and(|pan| pan.is_finite() && (-1.0..=1.0).contains(&pan)),
            "mute" | "solo" | "display" => value.is_boolean(),
            "fxPresetName" => value.is_string(),
            "fxParams" => {
                validate_effects(value)?;
                true
            }
            "scriptData" => true,
            _ => false,
        };
        if !supported {
            return Err(trf!("暂不支持混音设置 {} = {}", key, value));
        }
    }
    Ok(())
}

fn validate_effects(value: &Value) -> Result<(), String> {
    let effects = value.as_object().ok_or(tr!("混音 fxParams 必须为对象"))?;
    for (name, value) in effects {
        let label = match name.as_str() {
            "room" => tr!("空间定位"),
            "postRoomEq" => tr!("均衡器"),
            "compressor" => tr!("压缩器"),
            "reverb" => tr!("混响"),
            _ => return Err(trf!("暂不支持混音效果 fxParams.{}", name)),
        };
        match value.get("enabled").and_then(Value::as_bool) {
            Some(false) => {}
            Some(true) => {
                return Err(trf!("暂不支持已启用的{}效果（fxParams.{}）", label, name));
            }
            None => return Err(trf!("混音效果 fxParams.{}.enabled 缺失或不是布尔值", name)),
        }
    }
    Ok(())
}

pub(super) fn validate_export_bypass(config: &Extra) -> Result<(), String> {
    for (key, label) in [
        ("bypassPan", tr!("绕过声像")),
        ("bypassGain", tr!("绕过音量")),
        ("bypassEffects", tr!("绕过效果")),
    ] {
        match config.get(key) {
            None | Some(Value::Bool(false)) => {}
            Some(Value::Bool(true)) => return Err(trf!("暂不支持导出选项“{}”（{}）", label, key)),
            Some(_) => return Err(trf!("导出选项 {} 必须为布尔值", key)),
        }
    }
    Ok(())
}
