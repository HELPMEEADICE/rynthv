package org.rynthv.app

import android.content.Context
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.os.StatFs
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.util.Log
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.MessageDigest
import java.util.Locale
import java.util.UUID

class DocumentStore(context: Context) {
    private val resolver = context.contentResolver
    private val strings = context.resources
    private val root = File(context.filesDir, "RynthV/imports")
    private val shares = File(context.cacheDir, "shares")
    private val renders = File(context.cacheDir, "renders")

    fun importFile(uri: Uri, extensions: String): File {
        val rows = resolver.query(uri, COLUMNS, null, null, null)
            ?: throw IOException(strings.getString(R.string.document_name_unreadable))
        rows.use {
            if (!it.moveToFirst()) throw IOException(strings.getString(R.string.document_name_unreadable))
            val name = safeName(it.getString(0))
            if (extensions.isNotEmpty() && !hasExtension(name, extensions))
                throw IOException(strings.getString(R.string.file_type_unsupported, name))
            val bytes = it.getLong(1)
            val modified = it.getLong(2)
            val revision = if (bytes > 0 && modified > 0) hash("$uri\n$bytes\n$modified")
                else UUID.randomUUID().toString()
            return copyDocument(uri, File(File(root, "files"), revision), name, bytes, modified)
        }
    }

    fun importDirectory(tree: Uri): File {
        val destination = File(File(root, "trees"), hash(tree.toString()))
        ensureDirectory(destination)
        copyDirectory(tree, DocumentsContract.getTreeDocumentId(tree), destination, 0)
        return destination
    }

    private fun copyDirectory(tree: Uri, id: String, destination: File, depth: Int) {
        if (depth > 40) throw IOException(strings.getString(R.string.resource_directory_too_deep))
        val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id)
        val rows = resolver.query(children, CHILD_COLUMNS, null, null, null)
            ?: throw IOException(strings.getString(R.string.resource_directory_unreadable))
        rows.use {
            val names = HashSet<String>()
            while (it.moveToNext()) {
                val name = safeName(it.getString(0))
                if (!names.add(name)) throw IOException(strings.getString(R.string.resource_directory_duplicate_entry, name))
                val childId = it.getString(3)
                if (Document.MIME_TYPE_DIR == it.getString(4)) {
                    val next = File(destination, name)
                    ensureDirectory(next)
                    copyDirectory(tree, childId, next, depth + 1)
                } else {
                    val child = DocumentsContract.buildDocumentUriUsingTree(tree, childId)
                    copyDocument(child, destination, name, it.getLong(1), it.getLong(2))
                }
            }
        }
    }

    private fun copyDocument(uri: Uri, parent: File, name: String, bytes: Long, modified: Long): File {
        ensureDirectory(parent)
        val destination = File(parent, name)
        if (modified > 0 && bytes > 0 && destination.isFile &&
            destination.length() == bytes && destination.lastModified() == modified) return destination
        requireSpace(parent, bytes)
        val staging = File.createTempFile(".import-", ".part", parent)
        try {
            val descriptor = resolver.openFileDescriptor(uri, "r")
                ?: throw IOException(strings.getString(R.string.document_cannot_open, name))
            ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { input ->
                FileOutputStream(staging).use { output ->
                    copy(input, output, parent)
                    output.fd.sync()
                }
            }
            if (bytes > 0 && staging.length() != bytes)
                throw IOException(strings.getString(R.string.document_changed_while_reading, name))
            if (modified > 0) staging.setLastModified(modified)
            Files.move(staging.toPath(), destination.toPath(), StandardCopyOption.ATOMIC_MOVE,
                StandardCopyOption.REPLACE_EXISTING)
            return destination
        } finally { Files.deleteIfExists(staging.toPath()) }
    }

    fun publish(source: File, uri: Uri) {
        try {
            FileInputStream(source).use { input ->
                val descriptor = resolver.openFileDescriptor(uri, "wt")
                    ?: throw IOException(strings.getString(R.string.document_target_cannot_open))
                ParcelFileDescriptor.AutoCloseOutputStream(descriptor).use { output -> copy(input, output, null) }
            }
        } catch (error: Exception) {
            if (error !is IOException && error !is RuntimeException) throw error
            try { DocumentsContract.deleteDocument(resolver, uri) }
            catch (deletionError: Exception) { error.addSuppressed(deletionError) }
            throw error
        }
    }

    fun prepareShare(source: File): File {
        val directory = File(shares, UUID.randomUUID().toString())
        ensureDirectory(directory)
        val destination = File(directory, safeName(source.name))
        requireSpace(directory, source.length())
        Files.copy(source.toPath(), destination.toPath())
        return destination
    }

    fun pruneTransientFiles() {
        val cutoff = System.currentTimeMillis() - 3L * 24 * 60 * 60 * 1000
        pruneTransientFiles(shares, cutoff)
        pruneTransientFiles(renders, cutoff)
    }

    private fun copy(input: FileInputStream, output: FileOutputStream, volume: File?) {
        val buffer = ByteArray(64 * 1024)
        var untilCheck = 0
        while (true) {
            val count = input.read(buffer)
            if (count == -1) break
            if (volume != null && untilCheck <= 0) {
                requireSpace(volume, buffer.size.toLong())
                untilCheck = 8 * 1024 * 1024
            }
            output.write(buffer, 0, count)
            untilCheck -= count
        }
    }

    private fun requireSpace(volume: File, bytes: Long) {
        if (StatFs(volume.path).availableBytes - RESERVE < maxOf(0L, bytes))
            throw IOException(strings.getString(R.string.storage_insufficient))
    }

    private fun ensureDirectory(directory: File) { Files.createDirectories(directory.toPath()) }

    private fun safeName(name: String?): String {
        if (name.isNullOrEmpty() || name == "." || name == ".." ||
            '/' in name || '\\' in name || '\u0000' in name)
            throw IOException(strings.getString(R.string.document_name_invalid))
        return name
    }

    private fun hasExtension(name: String, extensions: String): Boolean {
        val lower = name.lowercase(Locale.ROOT)
        return extensions.split(',').any { lower.endsWith(".$it") }
    }

    private fun hash(text: String): String = MessageDigest.getInstance("SHA-256")
        .digest(text.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(Locale.ROOT, it.toInt() and 255) }

    companion object {
        private const val RESERVE = 32L * 1024 * 1024
        private val COLUMNS = arrayOf(Document.COLUMN_DISPLAY_NAME, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED)
        private val CHILD_COLUMNS = arrayOf(Document.COLUMN_DISPLAY_NAME, Document.COLUMN_SIZE,
            Document.COLUMN_LAST_MODIFIED, Document.COLUMN_DOCUMENT_ID, Document.COLUMN_MIME_TYPE)

        private fun pruneTransientFiles(root: File, cutoff: Long) {
            val folders = root.listFiles { path -> path.isDirectory && path.lastModified() < cutoff } ?: return
            for (folder in folders) {
                val files = folder.listFiles() ?: continue
                try {
                    for (file in files) if (file.isFile) Files.deleteIfExists(file.toPath())
                    Files.deleteIfExists(folder.toPath())
                } catch (error: IOException) { Log.w("RynthV", "Could not prune temporary exports", error) }
            }
        }
    }
}
