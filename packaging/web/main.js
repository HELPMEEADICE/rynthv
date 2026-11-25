import { set_wasm_module } from "./browser.js";

const status = document.querySelector("#status");
const canvas = document.querySelector("#editor");

async function start() {
    if (!isSecureContext) throw new Error("请使用 HTTPS 或 localhost 打开 RynthV。");
    if (typeof WebAssembly.Suspending !== "function" || typeof WebAssembly.promising !== "function") {
        throw new Error("此浏览器缺少 WebAssembly JS Promise Integration（JSPI）。请使用较新版本的 Chromium 浏览器。");
    }
    if (!navigator.storage?.getDirectory || !globalThis.indexedDB) {
        throw new Error("需要浏览器的 OPFS 和 IndexedDB 存储权限。");
    }
    const [module, { default: init, BrowserEditor }] = await Promise.all([
        WebAssembly.compileStreaming(fetch(new URL("./rynthv_bg.wasm", import.meta.url))),
        import("./rynthv.js"),
    ]);
    set_wasm_module(module);
    await init({ module_or_path: module });
    const editor = new BrowserEditor();
    await editor.start(canvas);
    status.remove();
    window.addEventListener("pagehide", event => {
        if (!event.persisted) editor.destroy();
    });
}

start().catch(error => {
    status.textContent = `无法启动 RynthV。${error?.message ?? error}`;
});
