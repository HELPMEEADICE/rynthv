// Strings shown by the web shell before the WebAssembly editor boots.
// Once Rust takes over, the interface language comes from the preferences.

const STRINGS = {
    "zh-CN": {
        canvas: "RynthV 歌声编辑器",
        loading: "正在加载 RynthV…",
        insecure: "请使用 HTTPS 或 localhost 打开 RynthV。",
        jspi: "此浏览器缺少 WebAssembly JS Promise Integration（JSPI）。请使用较新版本的 Chromium 浏览器。",
        storage: "需要浏览器的 OPFS 和 IndexedDB 存储权限。",
        failed: "无法启动 RynthV。",
    },
    "zh-TW": {
        canvas: "RynthV 歌聲編輯器",
        loading: "正在載入 RynthV…",
        insecure: "請使用 HTTPS 或 localhost 開啟 RynthV。",
        jspi: "此瀏覽器缺少 WebAssembly JS Promise Integration（JSPI）。請使用較新版本的 Chromium 瀏覽器。",
        storage: "需要瀏覽器的 OPFS 和 IndexedDB 儲存權限。",
        failed: "無法啟動 RynthV。",
    },
    en: {
        canvas: "RynthV singing voice editor",
        loading: "Loading RynthV…",
        insecure: "Open RynthV over HTTPS or from localhost.",
        jspi: "This browser lacks WebAssembly JS Promise Integration (JSPI). Use a recent Chromium browser.",
        storage: "RynthV needs OPFS and IndexedDB storage access.",
        failed: "RynthV could not start.",
    },
    ja: {
        canvas: "RynthV 歌声エディター",
        loading: "RynthV を読み込んでいます…",
        insecure: "RynthV は HTTPS または localhost で開いてください。",
        jspi: "このブラウザには WebAssembly JS Promise Integration（JSPI）がありません。新しい Chromium 系ブラウザを使用してください。",
        storage: "ブラウザの OPFS と IndexedDB へのアクセス権が必要です。",
        failed: "RynthV を起動できません。",
    },
};

function detect() {
    const tag = (navigator.languages?.[0] ?? navigator.language ?? "").toLowerCase();
    if (tag.startsWith("ja")) return STRINGS.ja;
    if (tag === "zh-tw" || tag === "zh-hk" || tag === "zh-hant" || tag.startsWith("zh-hant-")) return STRINGS["zh-TW"];
    if (tag.startsWith("zh")) return STRINGS["zh-CN"];
    return STRINGS.en;
}

export const strings = detect();
