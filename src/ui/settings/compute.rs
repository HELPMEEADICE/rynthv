use super::widgets::{row, section};
#[cfg(any(target_os = "android", target_arch = "wasm32"))]
use crate::trf;
use crate::{
    app::App,
    synthesis::inference::compute::{AdapterId, Backend},
    tr,
};
use eframe::egui::{ComboBox, Label, Ui};

const BACKENDS: [(Backend, &str); 3] = [
    (Backend::Auto, "自动选择（Auto）"),
    (Backend::WebGpu, "GPU（WebGPU）"),
    (Backend::Cpu, "CPU"),
];

pub(super) fn settings(app: &mut App, ui: &mut Ui) {
    section(ui, tr!("模型推理"));
    let current = app.synthesis.library.settings.compute_backend;
    let mut selected = current;
    row(ui, tr!("推理计算设备"), |ui| {
        let label = BACKENDS
            .iter()
            .find(|(backend, _)| *backend == current)
            .unwrap()
            .1;
        ComboBox::from_id_salt("inference-device")
            .width(ui.available_width())
            .selected_text(tr!(label))
            .show_ui(ui, |ui| {
                for (value, label) in BACKENDS {
                    ui.selectable_value(&mut selected, value, tr!(label));
                }
            });
    });
    let adapter = adapter_picker(app, ui, selected);
    app.synthesis.set_compute_settings(selected, adapter);

    let Some(diagnostics) = app.synthesis.compute_diagnostics() else {
        ui.weak(tr!("正在初始化推理设备…"));
        return;
    };
    let status = &diagnostics.status;
    #[cfg(not(target_arch = "wasm32"))]
    let actual = diagnostics.actual;
    #[cfg(target_arch = "wasm32")]
    let actual = diagnostics.actual.as_str();
    let execution = match actual {
        "cpu" => "CPU",
        "webgpu" => "WebGPU",
        "mixed" => "WebGPU + CPU",
        _ => tr!("尚未执行推理"),
    };
    row(ui, tr!("实际执行"), |ui| {
        ui.label(execution);
    });
    #[cfg(target_arch = "wasm32")]
    {
        let count = |key: &str| {
            diagnostics
                .activity
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default()
        };
        let milliseconds = |key| {
            diagnostics
                .activity
                .get(key)
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_default()
        };
        ui.weak(trf!(
            "GPU 提交 {}/{} 次／{} ms · 回读 {} 次／{} ms",
            count("completed_gpu_submissions"),
            count("gpu_submissions"),
            format!("{:.1}", milliseconds("submit_ms")),
            count("readback_count"),
            format!("{:.1}", milliseconds("readback_ms")),
        ));
        ui.collapsing(tr!("算子与缓存诊断"), |ui| {
            for key in [
                "gpu_dispatches",
                "cpu_operators",
                "weight_hits",
                "pipeline_hits",
                "peak_weight_cache_bytes",
            ] {
                if let Some(value) = diagnostics.activity.get(key) {
                    ui.label(format!("{key}  {value}"));
                }
            }
        });
    }
    if let Some(device) = &status.device {
        row(ui, tr!("当前显卡"), |ui| {
            ui.add(Label::new(device.id.to_string()).truncate());
        });
        #[cfg(target_os = "android")]
        ui.weak(trf!(
            "{} {} · {} MiB / 存储缓冲区",
            device.driver,
            device.driver_info,
            format!(
                "{:.0}",
                device.max_storage_binding_bytes as f64 / (1024.0 * 1024.0)
            ),
        ));
    }
    let initialization = match (status.requested, &status.device, &status.error) {
        (_, Some(_), None) => tr!("已就绪 · 支持计算"),
        (Backend::Cpu, _, _) => tr!("CPU 已就绪"),
        (Backend::Auto, _, Some(_)) => tr!("GPU 不可用 · 使用 CPU"),
        _ => tr!("WebGPU 计算失败"),
    };
    row(ui, tr!("推理设备状态"), |ui| {
        ui.label(initialization);
    });
    if let Some(error) = &status.error {
        ui.weak(error);
    }
}

fn adapter_picker(app: &App, ui: &mut Ui, backend: Backend) -> Option<AdapterId> {
    const DEFAULT: &str = "自动选择（独显优先）";

    let mut selected = app.synthesis.library.settings.compute_adapter.clone();
    let adapters = app.synthesis.compute_adapters();
    row(ui, tr!("WebGPU 显卡"), |ui| {
        ui.add_enabled_ui(backend != Backend::Cpu && adapters.is_some(), |ui| {
            let label = selected
                .as_ref()
                .map_or_else(|| tr!(DEFAULT).to_string(), ToString::to_string);
            ComboBox::from_id_salt("inference-adapter")
                .width(ui.available_width())
                .truncate()
                .selected_text(&label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected, None, tr!(DEFAULT));
                    for info in adapters
                        .unwrap_or_default()
                        .iter()
                        .filter(|info| info.is_selectable())
                    {
                        ui.selectable_value(
                            &mut selected,
                            Some(info.id.clone()),
                            info.id.to_string(),
                        );
                    }
                })
                .response
                .on_hover_text(label);
        });
    });
    selected
}
