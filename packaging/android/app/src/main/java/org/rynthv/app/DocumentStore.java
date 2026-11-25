package org.rynthv.app;

import android.content.ContentResolver;
import android.content.Context;
import android.database.Cursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.os.StatFs;
import android.provider.DocumentsContract;
import android.provider.DocumentsContract.Document;
import android.util.Log;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.Locale;
import java.util.UUID;

final class DocumentStore {
    private static final long RESERVE = 32L * 1024 * 1024;
    private static final String[] COLUMNS = {
        Document.COLUMN_DISPLAY_NAME, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED
    };
    private static final String[] CHILD_COLUMNS = {
        Document.COLUMN_DISPLAY_NAME, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED,
        Document.COLUMN_DOCUMENT_ID, Document.COLUMN_MIME_TYPE
    };

    private final ContentResolver resolver;
    private final File root;
    private final File shares;
    private final File renders;

    DocumentStore(Context context) {
        resolver = context.getContentResolver();
        root = new File(context.getFilesDir(), "RynthV/imports");
        shares = new File(context.getCacheDir(), "shares");
        renders = new File(context.getCacheDir(), "renders");
    }

    File importFile(Uri uri, String extensions) throws IOException {
        try (Cursor rows = resolver.query(uri, COLUMNS, null, null, null)) {
            if (rows == null || !rows.moveToFirst()) throw new IOException("无法读取文档名称");
            String name = safeName(rows.getString(0));
            if (!extensions.isEmpty() && !hasExtension(name, extensions)) {
                throw new IOException("不支持此文件类型：" + name);
            }
            long bytes = rows.getLong(1);
            long modified = rows.getLong(2);
            String revision = bytes > 0 && modified > 0
                ? hash(uri + "\n" + bytes + "\n" + modified) : UUID.randomUUID().toString();
            File parent = new File(new File(root, "files"), revision);
            return copyDocument(uri, parent, name, bytes, modified);
        }
    }

    File importDirectory(Uri tree) throws IOException {
        File destination = new File(new File(root, "trees"), hash(tree.toString()));
        ensureDirectory(destination);
        copyDirectory(tree, DocumentsContract.getTreeDocumentId(tree), destination, 0);
        return destination;
    }

    private void copyDirectory(Uri tree, String id, File destination, int depth) throws IOException {
        if (depth > 40) throw new IOException("资源目录层级过深");
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id);
        try (Cursor rows = resolver.query(children, CHILD_COLUMNS, null, null, null)) {
            if (rows == null) throw new IOException("无法读取资源目录");
            java.util.HashSet<String> names = new java.util.HashSet<>();
            while (rows.moveToNext()) {
                String name = safeName(rows.getString(0));
                if (!names.add(name)) throw new IOException("资源目录有同名条目：" + name);
                String childId = rows.getString(3);
                if (Document.MIME_TYPE_DIR.equals(rows.getString(4))) {
                    File next = new File(destination, name);
                    ensureDirectory(next);
                    copyDirectory(tree, childId, next, depth + 1);
                } else {
                    Uri child = DocumentsContract.buildDocumentUriUsingTree(tree, childId);
                    copyDocument(child, destination, name, rows.getLong(1), rows.getLong(2));
                }
            }
        }
    }

    private File copyDocument(Uri uri, File parent, String name, long bytes, long modified)
            throws IOException {
        ensureDirectory(parent);
        File destination = new File(parent, name);
        if (modified > 0 && bytes > 0 && destination.isFile()
                && destination.length() == bytes && destination.lastModified() == modified) {
            return destination;
        }
        requireSpace(parent, bytes);
        File staging = File.createTempFile(".import-", ".part", parent);
        try {
            ParcelFileDescriptor descriptor = resolver.openFileDescriptor(uri, "r");
            if (descriptor == null) throw new IOException("文档无法打开：" + name);
            try (FileInputStream input = new ParcelFileDescriptor.AutoCloseInputStream(descriptor);
                 FileOutputStream output = new FileOutputStream(staging)) {
                copy(input, output, parent);
                output.getFD().sync();
            }
            if (bytes > 0 && staging.length() != bytes) {
                throw new IOException("文档在读取期间变更：" + name);
            }
            if (modified > 0) staging.setLastModified(modified);
            Files.move(staging.toPath(), destination.toPath(), StandardCopyOption.ATOMIC_MOVE,
                StandardCopyOption.REPLACE_EXISTING);
            return destination;
        } finally {
            Files.deleteIfExists(staging.toPath());
        }
    }

    void publish(File source, Uri uri) throws IOException {
        try (FileInputStream input = new FileInputStream(source)) {
            ParcelFileDescriptor descriptor = resolver.openFileDescriptor(uri, "wt");
            if (descriptor == null) throw new IOException("无法打开目标文档");
            try (ParcelFileDescriptor.AutoCloseOutputStream output =
                    new ParcelFileDescriptor.AutoCloseOutputStream(descriptor)) {
                copy(input, output, null);
            }
        } catch (IOException | RuntimeException error) {
            try {
                DocumentsContract.deleteDocument(resolver, uri);
            } catch (Exception deletionError) {
                error.addSuppressed(deletionError);
            }
            throw error;
        }
    }

    File prepareShare(File source) throws IOException {
        File directory = new File(shares, UUID.randomUUID().toString());
        ensureDirectory(directory);
        File destination = new File(directory, safeName(source.getName()));
        requireSpace(directory, source.length());
        Files.copy(source.toPath(), destination.toPath());
        return destination;
    }

    void pruneTransientFiles() {
        long cutoff = System.currentTimeMillis() - 3L * 24 * 60 * 60 * 1000;
        pruneTransientFiles(shares, cutoff);
        pruneTransientFiles(renders, cutoff);
    }

    private static void pruneTransientFiles(File root, long cutoff) {
        File[] folders = root.listFiles(path -> path.isDirectory() && path.lastModified() < cutoff);
        if (folders == null) return;
        for (File folder : folders) {
            File[] files = folder.listFiles();
            if (files == null) continue;
            try {
                for (File file : files) {
                    if (file.isFile()) Files.deleteIfExists(file.toPath());
                }
                Files.deleteIfExists(folder.toPath());
            } catch (IOException error) {
                Log.w("RynthV", "Could not prune temporary exports", error);
            }
        }
    }

    private static void copy(FileInputStream input, FileOutputStream output, File volume)
            throws IOException {
        byte[] buffer = new byte[64 * 1024];
        int untilCheck = 0;
        for (int count; (count = input.read(buffer)) != -1;) {
            if (volume != null && untilCheck <= 0) {
                requireSpace(volume, buffer.length);
                untilCheck = 8 * 1024 * 1024;
            }
            output.write(buffer, 0, count);
            untilCheck -= count;
        }
    }

    private static void requireSpace(File volume, long bytes) throws IOException {
        if (new StatFs(volume.getPath()).getAvailableBytes() - RESERVE < Math.max(0, bytes)) {
            throw new IOException("应用存储空间不足");
        }
    }

    private static void ensureDirectory(File directory) throws IOException {
        Files.createDirectories(directory.toPath());
    }

    private static String safeName(String name) throws IOException {
        if (name == null || name.isEmpty() || name.equals(".") || name.equals("..")
                || name.indexOf('/') >= 0 || name.indexOf('\\') >= 0 || name.indexOf(0) >= 0) {
            throw new IOException("文档名称无效");
        }
        return name;
    }

    private static boolean hasExtension(String name, String extensions) {
        String lower = name.toLowerCase(Locale.ROOT);
        for (String extension : extensions.split(",")) {
            if (lower.endsWith("." + extension)) return true;
        }
        return false;
    }

    private static String hash(String text) {
        try {
            byte[] digest = MessageDigest.getInstance("SHA-256")
                .digest(text.getBytes(StandardCharsets.UTF_8));
            StringBuilder hex = new StringBuilder(digest.length * 2);
            for (byte value : digest) hex.append(String.format(Locale.ROOT, "%02x", value & 255));
            return hex.toString();
        } catch (NoSuchAlgorithmException impossible) {
            throw new AssertionError(impossible);
        }
    }
}
