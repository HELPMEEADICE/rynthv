// Origin-private files are opened only by the synthesis Worker.
let rootPromise;
let nextHandle = 1;
const handles = new Map();

function root() {
    return rootPromise ??= navigator.storage.getDirectory();
}

function parts(path) {
    const names = path.split("/").slice(1);
    if (!path.startsWith("/") || names.some(name => !name || name === "." || name === "..")) {
        if (path === "/") return [];
        throw new DOMException("Invalid OPFS path", "SyntaxError");
    }
    return names;
}

async function directory(names, create = false) {
    let handle = await root();
    for (const name of names) handle = await handle.getDirectoryHandle(name, { create });
    return handle;
}

async function parent(path) {
    const names = parts(path);
    const name = names.pop();
    if (!name) throw new DOMException("The storage root cannot be modified", "NotAllowedError");
    return [await directory(names), name];
}

async function entry(handle, name) {
    try {
        return await handle.getFileHandle(name);
    } catch (error) {
        if (error.name !== "TypeMismatchError") throw error;
        return handle.getDirectoryHandle(name);
    }
}

async function absent(handle, name) {
    try {
        await entry(handle, name);
        throw new DOMException(`Resource already exists: ${name}`, "AlreadyExistsError");
    } catch (error) {
        if (error.name !== "NotFoundError") throw error;
    }
}

export async function open_handle(path, write, create, exclusive, truncate) {
    const [handle, name] = await parent(path);
    if (exclusive) await absent(handle, name);
    const file = await handle.getFileHandle(name, { create });
    const access = await file.createSyncAccessHandle({ mode: write ? "readwrite" : "read-only" });
    try {
        if (truncate) access.truncate(0);
        const id = nextHandle++;
        handles.set(id, access);
        return id;
    } catch (error) {
        access.close();
        throw error;
    }
}

export function read_handle(id, bytes, at) { return handles.get(id).read(bytes, { at }); }
export function write_handle(id, bytes, at) { return handles.get(id).write(bytes, { at }); }
export function size_handle(id) { return handles.get(id).getSize(); }
export function flush_handle(id) { handles.get(id).flush(); }
export function close_handle(id) {
    const access = handles.get(id);
    handles.delete(id);
    access.close();
}

export async function stat_path(path) {
    const handle = path === "/" ? await root() : await entry(...await parent(path));
    return JSON.stringify({
        directory: handle.kind === "directory",
        size: handle.kind === "file" ? (await handle.getFile()).size : 0,
    });
}

export async function list_path(path) {
    const handle = await directory(parts(path));
    const entries = [];
    for await (const [name, child] of handle.entries()) entries.push([name, child.kind === "directory"]);
    return JSON.stringify(entries);
}

export async function make_directory(path, recursive) {
    if (recursive) return directory(parts(path), true);
    const [handle, name] = await parent(path);
    await absent(handle, name);
    await handle.getDirectoryHandle(name, { create: true });
}

export async function remove_path(path, isDirectory, recursive) {
    const [handle, name] = await parent(path);
    if (isDirectory) await handle.getDirectoryHandle(name);
    else await handle.getFileHandle(name);
    await handle.removeEntry(name, { recursive });
}

async function copyDirectory(source, target) {
    for await (const [name, child] of source.entries()) {
        if (child.kind === "directory") {
            await copyDirectory(child, await target.getDirectoryHandle(name, { create: true }));
        } else {
            const destination = await target.getFileHandle(name, { create: true });
            await (await child.getFile()).stream().pipeTo(await destination.createWritable());
        }
    }
}

export async function move_path(sourcePath, targetPath, replace) {
    const [sourceParent, sourceName] = await parent(sourcePath);
    const [targetParent, targetName] = await parent(targetPath);
    const source = await entry(sourceParent, sourceName);
    if (!replace || source.kind === "directory") await absent(targetParent, targetName);
    if (source.kind === "file" && typeof source.move === "function") {
        await source.move(targetParent, targetName);
        return;
    }
    if (source.kind === "file") {
        const destination = await targetParent.getFileHandle(targetName, { create: true });
        await (await source.getFile()).stream().pipeTo(await destination.createWritable());
    } else {
        const destination = await targetParent.getDirectoryHandle(targetName, { create: true });
        try {
            await copyDirectory(source, destination);
        } catch (error) {
            await targetParent.removeEntry(targetName, { recursive: true });
            throw error;
        }
    }
    await sourceParent.removeEntry(sourceName, { recursive: true });
}
