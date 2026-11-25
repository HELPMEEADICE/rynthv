let rootPromise;
let databasePromise;
const dropped = [];

function root() {
    return rootPromise ??= navigator.storage.getDirectory();
}

async function directory(parts, create = false) {
    let handle = await root();
    for (const name of parts) handle = await handle.getDirectoryHandle(name, { create });
    return handle;
}

function names(path) {
    const parts = path.split("/").filter(Boolean);
    if (parts[0] !== "RynthV" || parts.some(name => name === "." || name === ".." || name.includes("\\"))) {
        throw new Error("无效的浏览器资源路径");
    }
    return parts;
}

async function fileHandle(path, create = false) {
    const parts = names(path);
    const name = parts.pop();
    return (await directory(parts, create)).getFileHandle(name, { create });
}

async function stage(files, folder) {
    if (!files.length) return [];
    const id = crypto.randomUUID();
    const imports = await directory(["RynthV", "imports"], true);
    const prefix = `/RynthV/imports/${id}`;
    const paths = [];
    try {
        if (folder) {
            for (const file of files) {
                const path = `${prefix}/${file.webkitRelativePath}`;
                await file.stream().pipeTo(await (await fileHandle(path, true)).createWritable());
            }
            paths.push(`${prefix}/${files[0].webkitRelativePath.split("/")[0]}`);
        } else {
            const grouped = files.some(file => file.name.toLowerCase().endsWith(".svp"));
            if (grouped && new Set(files.map(file => file.name)).size !== files.length) {
                throw new Error("同一批工程文件中有重名文件，请分别导入。");
            }
            for (let index = 0; index < files.length; index++) {
                const file = files[index];
                const path = `${prefix}/${grouped || files.length === 1 ? "" : `${index}/`}${file.name}`;
                await file.stream().pipeTo(await (await fileHandle(path, true)).createWritable());
                paths.push(path);
            }
        }
    } catch (error) {
        try { await imports.removeEntry(id, { recursive: true }); } catch { /* Preserve the import error. */ }
        throw error;
    }
    return paths;
}

export function pick_files(kind, accept, title) {
    return new Promise((resolve, reject) => {
        const picker = document.createElement("input");
        picker.type = "file";
        picker.hidden = true;
        picker.title = title;
        picker.accept = accept;
        picker.multiple = kind === 1;
        picker.webkitdirectory = kind === 2;
        document.body.append(picker);
        picker.addEventListener("change", () => {
            const files = Array.from(picker.files);
            picker.remove();
            stage(files, kind === 2).then(paths => resolve(JSON.stringify(paths)), reject);
        }, { once: true });
        picker.addEventListener("cancel", () => {
            picker.remove();
            resolve("[]");
        }, { once: true });
        try { picker.showPicker(); } catch (error) { picker.remove(); reject(error); }
    });
}

export function install_drop(canvas) {
    canvas.addEventListener("dragover", event => event.preventDefault());
    canvas.addEventListener("drop", event => {
        event.preventDefault();
        event.stopImmediatePropagation();
        const files = Array.from(event.dataTransfer.files);
        stage(files, false).then(paths => dropped.push({ paths }), error => dropped.push({ error: String(error) }));
    }, { capture: true });
}

export function take_drop() { return dropped.length ? JSON.stringify(dropped.shift()) : undefined; }

export async function read_document(path, limit) {
    const file = await (await fileHandle(path)).getFile();
    if (file.size > limit) throw new Error("文件超过项目大小限制");
    return new Uint8Array(await file.arrayBuffer());
}

export async function delete_document(path) {
    const parts = names(path);
    const name = parts.pop();
    await (await directory(parts)).removeEntry(name);
    if (parts[1] === "imports" && parts.length >= 3) {
        const last = parts.pop();
        try { await (await directory(parts)).removeEntry(last); } catch (error) {
            if (error.name !== "InvalidModificationError") throw error;
        }
    }
}

function download(blob, name) {
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = name;
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

export function download_bytes(bytes, name, type) { download(new Blob([bytes], { type }), name); }

export async function download_file(path) {
    const file = await (await fileHandle(path)).getFile();
    download(file, file.name);
}

export async function clear_exports() {
    const library = await directory(["RynthV"], true);
    try {
        await library.removeEntry("exports", { recursive: true });
    } catch (error) {
        if (error.name !== "NotFoundError") throw error;
    }
}

function db() {
    return databasePromise ??= new Promise((resolve, reject) => {
        const request = indexedDB.open("RynthV", 1);
        request.onupgradeneeded = () => request.result.createObjectStore("documents");
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => { databasePromise = undefined; reject(request.error); };
    });
}

async function transaction(mode, action) {
    const database = await db();
    return new Promise((resolve, reject) => {
        const transaction = database.transaction("documents", mode);
        const request = action(transaction.objectStore("documents"));
        transaction.oncomplete = () => resolve(request.result);
        transaction.onabort = () => reject(request.error ?? transaction.error);
    });
}

export function read_record(key) { return transaction("readonly", store => store.get(key)); }
export function write_record(key, value) { return transaction("readwrite", store => store.put(value, key)); }

export async function save_project(id, name, snapshot) {
    const database = await db();
    return new Promise((resolve, reject) => {
        const transaction = database.transaction("documents", "readwrite");
        const store = transaction.objectStore("documents");
        const index = store.get("project-index");
        index.onsuccess = () => {
            const projects = (index.result ?? []).filter(project => project.id !== id);
            projects.unshift({ id, name });
            store.put(projects, "project-index");
            store.put(snapshot, `project:${id}`);
            store.put(snapshot, "recovery");
        };
        transaction.oncomplete = () => resolve();
        transaction.onabort = () => reject(transaction.error);
    });
}

export async function project_index() {
    return JSON.stringify(await read_record("project-index") ?? []);
}

export async function fetch_font() {
    const response = await fetch(new URL("./NotoSansCJKsc-Regular.otf", import.meta.url));
    if (!response.ok) throw new Error(`无法加载编辑器字体 (${response.status})`);
    return new Uint8Array(await response.arrayBuffer());
}
