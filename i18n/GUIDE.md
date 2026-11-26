# i18n 改写规范（subagent 必读）

## 已冻结的共享契约（不要修改这些文件）

- src/i18n.rs — Locale / tr() / fill() / tr! / trf! / 目录加载
- src/ui/settings/preferences.rs — Preferences.locale
- src/ui/theme/fonts.rs, src/ui/theme/fonts/linux.rs — 按语言加载字体
- src/lib.rs — pub mod i18n;
- src/ui/settings/pages.rs 的 general() 语言选择器（已改，勿动这一段）

## 你要做的事

1. 在你负责的文件里，把所有**面向用户**的简体中文字符串字面量改写成 tr!/trf! 调用。
2. 把你用到的每个 msgid（就是原简体中文原文，逐字不改）连同三语译文写入你负责的 TSV 分片。

## 调用形式

- 无占位符：tr!("保存")
- 带占位符：trf!("音轨 {}：{error}", name, error)  ← 错误写法，见下
  - 占位符只能是 {} （顺序取参）或 {0}/{1} （显式下标），译文可据此重排语序。
  - 具名参数（{error} 这种）**不能**用于 trf!，必须改成位置占位符 {} 并把值作为实参传入：
    trf!("音轨 {}：{}", name, error)
  - 参数一律作为额外实参传入，不要先 format! 再 tr!。

## 导入

- 文件内若无 use，加 use crate::{tr, trf}; 或 use crate::tr;
- 宏由 #[macro_export] 导出，从 crate 根路径调用。

## 硬性约束

- **msgid 逐字不变**：TSV 第一列必须与 Rust 里传给 tr! 的字符串完全一致（含全角标点、空格）。
- **const 只存 msgid**：const X: &str 里放简体原文，取译文要在使用点 tr!(X)。
- **不要翻译**：文件路径、扩展名、serde 字段名、id_salt/persistent_id、快捷键名（Ctrl+N）、协议/错误码（如 INPUT_READ）、给开发者看的内部日志、以及**用户项目数据**（音轨名、歌词、声库名）。
- **不要动**：标识符、枚举变体名；给开发者看的注释可以保留中文。
- **tr! 只接受 &'static str 字面量或 const &'static str**；动态拼接的 String 必须先得到 msgid 模板再 trf!。
- 除字符串外一切逻辑不变；不要重排代码结构，不要顺带重构。

## TSV 分片格式

文件位于 i18n/<locale>/<group>.tsv，locale 为 zh-TW / en / ja 之一。

- 每行 <简体中文 msgid> TAB <译文>，必须真的用一个 TAB 分隔。
- 以 # 开头的行是注释，空行忽略。
- 转义：\n \t \r \\ 。msgid 内不能含裸 TAB 或裸换行。
- UTF-8 无 BOM，LF 换行。
- 占位符 {} / {0} 在译文里必须保留同样的个数与下标。
- 只写你负责文件里实际用到的 msgid，不要重复。

## 译文要求

- **zh-TW**：台湾正体，用词按台湾习惯（檔案／設定／音軌／匯出／儲存／開啟／網路／最佳化）。注意简繁一对多：发→發/髮、干→乾/幹、后→後/后、里→裡。标点用全角。
- **en**：自然简洁的控件风格（Save / Open / Export to File），用 DAW 与歌声合成领域通行术语（pitch, timbre, phoneme, retake, vocoder, voicebank, note group）。
- **ja**：自然技术日文 UI 体（〜を保存／〜を開く），术语用 音高／音色／音素／リテイク／声質／ボーカロイド。
- 三语译文必须与简体原文语义等价，不增删信息。
