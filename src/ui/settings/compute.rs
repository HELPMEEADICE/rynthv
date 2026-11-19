use super::widgets::{row, section};
use crate::{app::App, synthesis::inference::compute::Backend};
use eframe::egui::{ComboBox, Ui};

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
    if selected != current {
        app.synthesis.set_compute_backend(selected);
    }
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
        row(ui, "GPU", |ui| {
            ui.label(format!("{} · {}", device.name, device.api));
        });
    }
    let initialization = match (status.requested, &status.device, &status.error) {
        (_, Some(_), None) => "已就绪 · 支持计算",
        (Backend::Cpu, _, _) => "CPU 已就绪",
        (Backend::Auto, None, Some(_)) => "GPU 不可用 · 使用 CPU",
        _ => "WebGPU 计算失败",
    };
    row(ui, "推理设备状态", |ui| {
        ui.label(initialization);
    });
    if let Some(error) = &status.error {
        ui.weak(error);
    }
}
