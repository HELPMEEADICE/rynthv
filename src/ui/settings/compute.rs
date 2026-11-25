use super::widgets::{row, section};
use crate::{
    app::App,
    synthesis::inference::compute::{AdapterId, Backend},
};
use eframe::egui::{ComboBox, Label, Ui};

const BACKENDS: [(Backend, &str); 3] = [
    (Backend::Auto, "自动选择（Auto）"),
    (Backend::WebGpu, "GPU（WebGPU）"),
    (Backend::Cpu, "CPU"),
];

pub(super) fn settings(app: &mut App, ui: &mut Ui) {
    section(ui, "模型推理");
    let current = app.synthesis.library.settings.compute_backend;
    let mut selected = current;
    row(ui, "推理计算设备", |ui| {
        let label = BACKENDS
            .iter()
            .find(|(backend, _)| *backend == current)
            .unwrap()
            .1;
        ComboBox::from_id_salt("inference-device")
            .width(ui.available_width())
            .selected_text(label)
            .show_ui(ui, |ui| {
                for (value, label) in BACKENDS {
                    ui.selectable_value(&mut selected, value, label);
                }
            });
    });
    let adapter = adapter_picker(app, ui, selected);
    app.synthesis.set_compute_settings(selected, adapter);

    let Some(diagnostics) = app.synthesis.compute_diagnostics() else {
        ui.weak("正在初始化推理设备…");
        return;
    };
    let status = &diagnostics.status;
    let execution = match diagnostics.actual {
        "cpu" => "CPU",
        "webgpu" => "WebGPU",
        "mixed" => "WebGPU + CPU",
        _ => "尚未执行推理",
    };
    row(ui, "实际执行", |ui| {
        ui.label(execution);
    });
    if let Some(device) = &status.device {
        row(ui, "当前显卡", |ui| {
            ui.add(Label::new(device.id.to_string()).truncate());
        });
        #[cfg(target_os = "android")]
        ui.weak(format!(
            "{} {} · {:.0} MiB / 存储缓冲区",
            device.driver,
            device.driver_info,
            device.max_storage_binding_bytes as f64 / (1024.0 * 1024.0),
        ));
    }
    let initialization = match (status.requested, &status.device, &status.error) {
        (_, Some(_), None) => "已就绪 · 支持计算",
        (Backend::Cpu, _, _) => "CPU 已就绪",
        (Backend::Auto, _, Some(_)) => "GPU 不可用 · 使用 CPU",
        _ => "WebGPU 计算失败",
    };
    row(ui, "推理设备状态", |ui| {
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
    row(ui, "WebGPU 显卡", |ui| {
        ui.add_enabled_ui(backend != Backend::Cpu && adapters.is_some(), |ui| {
            let label = selected
                .as_ref()
                .map_or_else(|| DEFAULT.into(), ToString::to_string);
            ComboBox::from_id_salt("inference-adapter")
                .width(ui.available_width())
                .truncate()
                .selected_text(&label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected, None, DEFAULT);
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
