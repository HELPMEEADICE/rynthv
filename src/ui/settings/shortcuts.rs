use super::widgets::section;
use crate::{tr, ui::theme::*};
use eframe::egui::*;

pub(super) fn show(ui: &mut Ui) {
    ui.weak(tr!("当前快捷键；自定义绑定尚未接入。"));
    for (title, entries) in [
        (
            tr!("工程"),
            &[
                (tr!("新建"), "Ctrl+N"),
                (tr!("打开"), "Ctrl+O"),
                (tr!("保存"), "Ctrl+S"),
                (tr!("另存为"), "Ctrl+Shift+S"),
            ][..],
        ),
        (
            tr!("编辑"),
            &[
                (tr!("撤销"), "Ctrl+Z"),
                (tr!("重做"), "Ctrl+Y / Ctrl+Shift+Z"),
                (tr!("全选音符"), "Ctrl+A"),
                (tr!("复制 / 剪切 / 粘贴"), "Ctrl+C / X / V"),
                (tr!("重复音符"), "Ctrl+D"),
                (tr!("删除音符"), "Delete"),
                (tr!("取消选择"), "Esc"),
                (tr!("填入歌词"), "Ctrl+L"),
                (tr!("静音音符"), "Ctrl+M"),
                (tr!("重录新的采样"), "Alt+T"),
                (tr!("裁剪保留使用采样"), "Shift+Alt+T"),
                (tr!("升高 / 降低八度"), "Ctrl+↑ / ↓"),
                (tr!("音符面板"), "Ctrl+B"),
            ][..],
        ),
        (
            tr!("走带控制"),
            &[
                (tr!("播放 / 暂停"), "Space"),
                (tr!("停止"), "Ctrl+Space"),
                (tr!("跳到结尾"), "Ctrl+End"),
                (tr!("按网格移动走带"), "← / →"),
            ][..],
        ),
        (
            tr!("工具与视野"),
            &[
                (tr!("选择 / 铅笔工具"), "Alt+1 / Alt+2"),
                (tr!("创建音符 / 编辑歌词"), tr!("双击")),
                (tr!("平移视野"), tr!("中键拖动")),
                (tr!("水平滚动"), tr!("Shift+滚轮")),
                (tr!("缩放"), tr!("Ctrl+滚轮")),
            ][..],
        ),
    ] {
        section(ui, title);
        for (label, key) in entries {
            ui.horizontal(|ui| {
                ui.set_min_height(28.0);
                ui.label(*label);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    Frame::new()
                        .fill(button_color())
                        .corner_radius(3)
                        .inner_margin(Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.label(RichText::new(*key).color(text_color()));
                        });
                });
            });
        }
    }
}
