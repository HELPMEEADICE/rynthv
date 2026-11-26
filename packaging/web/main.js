import { set_wasm_module } from "./browser.js";
import { strings } from "./strings.js";

const status = document.querySelector("#status");
const canvas = document.querySelector("#editor");

canvas.setAttribute("aria-label", strings.canvas);
status.textContent = strings.loading;

async function start() {
    if (!isSecureContext) throw new Error(strings.insecure);
    if (typeof WebAssembly.Suspending !== "function" || typeof WebAssembly.promising !== "function") {
        throw new Error(strings.jspi);
    }
    if (!navigator.storage?.getDirectory || !globalThis.indexedDB) {
        throw new Error(strings.storage);
    }
    const [module, { default: init, BrowserEditor }] = await Promise.all([
        WebAssembly.compileStreaming(fetch(new URL("./rynthv_bg.wasm", import.meta.url))),
        import("./rynthv.js"),
    ]);
    await init({ module_or_path: module });
    set_wasm_module(module);
    const editor = new BrowserEditor();
    await editor.start(canvas);
    status.remove();
    window.addEventListener("pagehide", event => {
        if (!event.persisted) editor.destroy();
    });
}

start().catch(error => {
    status.textContent = `${strings.failed}${error?.message ?? error}`;
});
