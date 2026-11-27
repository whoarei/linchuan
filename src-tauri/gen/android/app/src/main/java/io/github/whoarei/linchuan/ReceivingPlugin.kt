package io.github.whoarei.linchuan

import android.Manifest
import android.app.Activity
import android.app.DownloadManager
import android.content.ContentValues
import android.content.Intent
import android.content.pm.PackageManager
import android.media.MediaScannerConnection
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.ParcelFileDescriptor
import android.os.StatFs
import android.os.storage.StorageManager
import android.system.Os
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import androidx.activity.result.ActivityResult
import androidx.core.content.FileProvider
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class ReceiveOptions {
  var directory: String = "@downloads"
  var strict: Boolean = false
  var replace: String? = null
  var sourcePath: String = ""
  var name: String = "file"
  var mime: String = "application/octet-stream"
  var location: String = ""
}

@TauriPlugin(permissions = [Permission(alias = "legacy-storage", strings = [Manifest.permission.WRITE_EXTERNAL_STORAGE])])
class ReceivingPlugin(private val activity: Activity) : Plugin(activity) {
  private val resolver get() = activity.contentResolver

  private fun localDirectory(directory: String): Pair<File, File> {
    val root: File
    val target: File
    if (directory == "@downloads") {
      root = Environment.getExternalStorageDirectory()
      target = File(root, "${Environment.DIRECTORY_DOWNLOADS}/邻传")
    } else {
      val uri = Uri.parse(directory)
      if (uri.authority != "com.android.externalstorage.documents")
        throw Exception("所选文档提供器无法查询本地磁盘空间，请选择本机存储目录")
      val id = DocumentsContract.getTreeDocumentId(uri)
      val volume = id.substringBefore(':')
      root = if (volume.equals("primary", true)) Environment.getExternalStorageDirectory()
        else if (Build.VERSION.SDK_INT >= 30) {
          activity.getSystemService(StorageManager::class.java).storageVolumes
            .firstOrNull { it.uuid.equals(volume, true) }?.directory ?: throw Exception("存储卷不可用")
        } else File("/storage/$volume")
      target = File(root, id.substringAfter(':', ""))
      val canonicalRoot = root.canonicalPath
      if (target.canonicalPath != canonicalRoot && !target.canonicalPath.startsWith(canonicalRoot + File.separator))
        throw Exception("目录不属于所选存储卷")
    }
    return Pair(root, target)
  }

  @Command
  fun storageInfo(invoke: Invoke) {
    try {
      val directory = invoke.parseArgs(ReceiveOptions::class.java).directory
      val (root, target) = localDirectory(directory)
      var existing = target
      while (!existing.isDirectory) existing = existing.parentFile ?: throw Exception("目标磁盘不可用")
      // Emulated shared storage and private app storage consume the same /data volume.
      val physical = if (Environment.isExternalStorageEmulated(root)) Environment.getDataDirectory() else existing
      invoke.resolve(JSObject().put("volume", "unix:${Os.stat(physical.absolutePath).st_dev}")
        .put("available", StatFs(existing.absolutePath).availableBytes).put("path", existing.absolutePath))
    } catch (e: Exception) { invoke.reject("无法检查目标剩余空间：${e.message}") }
  }

  private data class Entry(val location: String, val file: File?, val size: Long?, val stamp: String, val providerRename: Boolean) {
    val replaceable get() = providerRename || file?.let { it.isFile && it.canWrite() && it.parentFile?.canWrite() == true } == true
  }

  private fun findEntry(directory: String, name: String): Entry? {
    val folder = localDirectory(directory).second
    fun entry(location: String, disk: File, bytes: Long?, modified: Long, providerRename: Boolean = false): Entry {
      val file = disk.takeIf { it.exists() && it.canonicalFile.parentFile == folder.canonicalFile &&
        it.canonicalFile == File(folder.canonicalFile, it.name) }
      val stat = file?.let { Os.stat(it.absolutePath) }
      return Entry(location, file, bytes, "$location:$modified:$bytes:${stat?.st_ino}:${file?.lastModified()}", providerRename)
    }
    if (directory.startsWith("content://")) {
      val tree = Uri.parse(directory)
      val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
      val cols = arrayOf(DocumentsContract.Document.COLUMN_DOCUMENT_ID, OpenableColumns.DISPLAY_NAME,
        OpenableColumns.SIZE, DocumentsContract.Document.COLUMN_LAST_MODIFIED, DocumentsContract.Document.COLUMN_MIME_TYPE, DocumentsContract.Document.COLUMN_FLAGS)
      val cursor = resolver.query(children, cols, null, null, null) ?: throw Exception("无法检查所选目录")
      cursor.use {
        while (it.moveToNext()) {
          val actual = it.getString(1)
          if (actual.equals(name, true)) {
            val uri = DocumentsContract.buildDocumentUriUsingTree(tree, it.getString(0))
            val size = if (it.getString(4) == DocumentsContract.Document.MIME_TYPE_DIR || it.isNull(2)) null else it.getLong(2)
            val flags = it.getInt(5)
            val canRename = it.getString(4) != DocumentsContract.Document.MIME_TYPE_DIR &&
              flags and DocumentsContract.Document.FLAG_SUPPORTS_RENAME != 0 && flags and DocumentsContract.Document.FLAG_SUPPORTS_DELETE != 0
            return entry(uri.toString(), File(folder, actual), size, it.getLong(3), canRename)
          }
        }
      }
    } else if (Build.VERSION.SDK_INT >= 29) {
      val collection = MediaStore.Downloads.EXTERNAL_CONTENT_URI
      resolver.query(collection, arrayOf("_id", "_display_name", "_size", "date_modified", "_data"),
        "relative_path=? AND _display_name=?", arrayOf("${Environment.DIRECTORY_DOWNLOADS}/邻传/", name), null)?.use {
        if (it.moveToFirst()) {
          val uri = android.content.ContentUris.withAppendedId(collection, it.getLong(0))
          val disk = it.getString(4)?.let { path -> File(path) } ?: File(folder, it.getString(1))
          val writable = try { resolver.openFileDescriptor(uri, "rw")?.use { true } ?: false } catch (_: Exception) { false }
          return entry(uri.toString(), disk, it.getLong(2), it.getLong(3), writable)
        }
      }
    }
    val disk = File(folder, name)
    if (!disk.exists()) return null
    return entry(disk.absolutePath, disk, disk.length().takeIf { disk.isFile }, disk.lastModified())
  }

  @Command
  fun inspectFile(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(ReceiveOptions::class.java)
      val entry = findEntry(args.directory, args.name)
      val result = JSObject()
      if (entry != null) result.put("entry", JSObject().put("stamp", entry.stamp)
        .put("size", entry.size).put("replaceable", entry.replaceable).put("location", entry.location))
      invoke.resolve(result)
    } catch (e: Exception) { invoke.reject("无法检查同名文件：${e.message}") }
  }

  @Command
  fun beginFile(invoke: Invoke) {
    val args = invoke.parseArgs(ReceiveOptions::class.java)
    if (args.directory == "@downloads" && Build.VERSION.SDK_INT < 29 &&
      activity.checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) != PackageManager.PERMISSION_GRANTED) {
      requestPermissionForAlias("legacy-storage", invoke, "outputPermissionResult")
      return
    }
    Thread {
      var location: String? = null
      try {
        val descriptor: ParcelFileDescriptor
        val existing = if (args.strict) findEntry(args.directory, args.name) else null
        if (args.replace != null && (existing == null || !existing.replaceable || existing.stamp != args.replace))
          throw Exception("目标文件已变化，请重新接收")
        if (args.replace == null && existing != null) throw Exception("目标名称已被占用，请重新接收")
        val outputName = if (args.replace != null) temporaryName("part", args.name) else args.name
        if (args.replace != null && !existing!!.providerRename) {
          val temp = File.createTempFile(".linchuan-", ".part", existing.file!!.parentFile)
          location = temp.absolutePath
          descriptor = ParcelFileDescriptor.open(temp, ParcelFileDescriptor.MODE_WRITE_ONLY)
        } else if (args.directory == "@downloads" && Build.VERSION.SDK_INT >= 29) {
          val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, outputName)
            put(MediaStore.Downloads.MIME_TYPE, args.mime)
            put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/邻传")
            put(MediaStore.Downloads.IS_PENDING, 1)
          }
          val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: throw Exception("无法创建文件")
          location = uri.toString()
          if (args.strict && displayName(uri) != outputName) throw Exception("目标名称已被占用，请重新接收")
          descriptor = resolver.openFileDescriptor(uri, "w") ?: throw Exception("目标目录不可写")
        } else if (args.directory.startsWith("content://")) {
          val tree = Uri.parse(args.directory)
          val id = DocumentsContract.getTreeDocumentId(tree)
          val parent = DocumentsContract.buildDocumentUriUsingTree(tree, id)
          val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id)
          val names = mutableSetOf<String>()
          resolver.query(children, arrayOf(DocumentsContract.Document.COLUMN_DISPLAY_NAME), null, null, null)?.use {
            while (it.moveToNext()) names.add(it.getString(0))
          }
          val name = if (args.strict) outputName else (0..9999).asSequence().map { numbered(args.name, it) }.firstOrNull { it !in names }
            ?: throw Exception("同名文件过多")
          val uri = DocumentsContract.createDocument(resolver, parent, args.mime, name) ?: throw Exception("无法创建文件")
          location = uri.toString()
          if (args.strict && displayName(uri) != outputName) throw Exception("目标名称已被占用，请重新接收")
          descriptor = resolver.openFileDescriptor(uri, "w") ?: throw Exception("目标目录不可写")
        } else {
          val folder = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "邻传")
          if (!folder.isDirectory && !folder.mkdirs()) throw Exception("无法创建下载目录")
          val target = (0..(if (args.strict) 0 else 9999)).asSequence().map { File(folder, numbered(args.name, it)) }.firstOrNull { it.createNewFile() }
            ?: throw Exception("同名文件过多")
          location = target.absolutePath
          descriptor = ParcelFileDescriptor.open(target, ParcelFileDescriptor.MODE_WRITE_ONLY)
        }
        invoke.resolve(JSObject().put("fd", descriptor.detachFd()).put("location", location))
      } catch (e: Exception) {
        location?.let { try { removeLocation(it) } catch (_: Exception) {} }
        invoke.reject("无法创建目标文件：${e.message}")
      }
    }.start()
  }

  @PermissionCallback
  fun outputPermissionResult(invoke: Invoke) {
    if (activity.checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED) beginFile(invoke)
    else invoke.reject("未获得存储权限")
  }

  @Command
  fun completeFile(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(ReceiveOptions::class.java)
      val location = args.location
      if (args.replace != null) {
        val entry = findEntry(args.directory, args.name)
        if (entry == null || !entry.replaceable || entry.stamp != args.replace) throw Exception("目标文件已变化，请重新接收")
        if (location.startsWith("content://")) {
          val published = publishDocument(args, entry.location)
          invoke.resolve(result(args.name, published))
          return
        }
        val target = entry.file!!
        Os.rename(location, target.absolutePath)
        MediaScannerConnection.scanFile(activity, arrayOf(target.absolutePath), null, null)
        invoke.resolve(result(args.name, entry.location))
        return
      }
      val name = if (location.startsWith("content://")) {
        val uri = Uri.parse(location)
        if (uri.authority == "media" && Build.VERSION.SDK_INT >= 29) {
          if (resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null) != 1)
            throw Exception("无法完成文件保存")
        }
        displayName(uri) ?: "file"
      } else {
        val file = File(location)
        MediaScannerConnection.scanFile(activity, arrayOf(location), null, null)
        file.name
      }
      invoke.resolve(result(name, location))
    } catch (e: Exception) { invoke.reject("保存未完成：${e.message}") }
  }

  private fun temporaryName(kind: String, name: String): String {
    val extension = File(name).extension
    return "linchuan-$kind-${java.util.UUID.randomUUID()}" + if (extension.isEmpty()) "" else ".$extension"
  }

  private fun renameDocument(location: String, name: String, publish: Boolean = false): String {
    val uri = Uri.parse(location)
    val renamed = if (uri.authority == "media" && Build.VERSION.SDK_INT >= 29) {
      val values = ContentValues().apply {
        put(MediaStore.Downloads.DISPLAY_NAME, name)
        if (publish) put(MediaStore.Downloads.IS_PENDING, 0)
      }
      if (resolver.update(uri, values, null, null) != 1) throw Exception("下载文件改名失败")
      uri
    } else DocumentsContract.renameDocument(resolver, uri, name) ?: throw Exception("文档改名失败")
    if (displayName(renamed) != name) throw Exception("目标名称已被占用")
    return renamed.toString()
  }

  private val replacements get() = activity.getSharedPreferences("receive-replacements", Activity.MODE_PRIVATE)

  // Document providers cannot replace an existing name directly. Keep the original
  // nearby until the completed download has its final name, including across a restart.
  private fun publishDocument(args: ReceiveOptions, original: String): String {
    val backup = temporaryName("original", args.name)
    val record = org.json.JSONObject().put("directory", args.directory).put("name", args.name).put("backup", backup)
    if (!replacements.edit().putString(args.location, record.toString()).commit()) throw Exception("无法登记文件替换")
    val published = try {
      renameDocument(original, backup)
      renameDocument(args.location, args.name, true)
    } catch (e: Exception) {
      finishReplacement(args.location, record)
      throw e
    }
    record.put("published", published)
    replacements.edit().putString(args.location, record.toString()).commit()
    try { finishReplacement(args.location, record) }
    catch (e: Exception) { android.util.Log.w("Receiving", "稍后清理覆盖前的副本", e) }
    return published
  }

  private fun finishReplacement(location: String, record: org.json.JSONObject) {
    val directory = record.getString("directory")
    val name = record.getString("name")
    val backup = findEntry(directory, record.getString("backup"))
    if (backup != null) {
      val final = findEntry(directory, name)
      if (final == null) renameDocument(backup.location, name)
      else if (final.location == location || final.location == record.optString("published")) removeLocation(backup.location)
      else throw Exception("目标名称已被占用，原文件保存在 ${record.getString("backup")}")
    }
    replacements.edit().remove(location).commit()
  }

  @Command
  fun recoverReplacements(invoke: Invoke) {
    for ((location, record) in replacements.all) {
      try { finishReplacement(location, org.json.JSONObject(record as String)) }
      catch (e: Exception) { android.util.Log.w("Receiving", "原文件仍保留，稍后重试恢复", e) }
    }
    invoke.resolve(JSObject())
  }

  @Command
  fun abortFile(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(ReceiveOptions::class.java)
      // A MediaStore URI is stable across a rename. Recovery can still refer
      // to the staging URI just after the document has been published.
      if (args.replace != null && args.location.startsWith("content://")) {
        val current = displayName(Uri.parse(args.location))
        if (current == null || current == args.name) { invoke.resolve(JSObject()); return }
      }
      removeLocation(args.location)
      invoke.resolve(JSObject())
    } catch (e: Exception) { invoke.reject(e.message) }
  }

  @Command
  fun openInput(invoke: Invoke) {
    try {
      val location = invoke.parseArgs(ReceiveOptions::class.java).location
      val descriptor = if (location.startsWith("content://")) resolver.openFileDescriptor(Uri.parse(location), "r")
        else ParcelFileDescriptor.open(File(location), ParcelFileDescriptor.MODE_READ_ONLY)
      invoke.resolve(JSObject().put("fd", (descriptor ?: throw Exception("文件不可读")).detachFd()))
    } catch (e: Exception) { invoke.reject("原文件已移动、删除或无法读取：${e.message}") }
  }

  private fun removeLocation(location: String) {
    val removed = if (location.startsWith("content://")) {
      val uri = Uri.parse(location)
      if (DocumentsContract.isDocumentUri(activity, uri)) DocumentsContract.deleteDocument(resolver, uri)
      else resolver.delete(uri, null, null) > 0
    } else File(location).delete()
    if (!removed) throw Exception("原文件未能删除")
  }

  @Command
  fun deleteFile(invoke: Invoke) {
    try { removeLocation(invoke.parseArgs(ReceiveOptions::class.java).location); invoke.resolve(JSObject()) }
    catch (e: Exception) { invoke.reject(e.message) }
  }

  @Command
  fun describeDirectory(invoke: Invoke) {
    try {
      val directory = invoke.parseArgs(ReceiveOptions::class.java).directory
      val label = if (directory == "@downloads") "下载/邻传" else {
        val tree = Uri.parse(directory)
        displayName(DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree)))
          ?: "已选择的文件夹"
      }
      invoke.resolve(JSObject().put("label", label))
    } catch (e: Exception) { invoke.reject("目录授权已失效，请在设置中重新选择：${e.message}") }
  }

  @Command
  fun pickFiles(invoke: Invoke) {
    try {
      val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).setType("*/*").addCategory(Intent.CATEGORY_OPENABLE)
        .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true).putExtra(Intent.EXTRA_LOCAL_ONLY, true)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
      startActivityForResult(invoke, intent, "filesResult")
    } catch(e: Exception) { invoke.reject("无法打开文件选择器：" + e.message) }
  }

  @ActivityCallback
  fun filesResult(invoke: Invoke, result: ActivityResult) {
    try {
      val files = org.json.JSONArray()
      if(result.resultCode == Activity.RESULT_OK) {
        val data = result.data
        val uris = if(data?.clipData != null) (0 until data.clipData!!.itemCount).map { data.clipData!!.getItemAt(it).uri }
          else listOfNotNull(data?.data)
        if(uris.size > 64) throw Exception("一次最多发送 64 个文件")
        for(uri in uris) {
          val flags = (data?.flags ?: 0) and Intent.FLAG_GRANT_READ_URI_PERMISSION
          if(flags != 0 && ((data?.flags ?: 0) and Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION) != 0)
            resolver.takePersistableUriPermission(uri, flags)
          var size: Long? = null
          var name: String? = null
          resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use {
            if(it.moveToFirst()) { name=it.getString(0); if(!it.isNull(1)) size=it.getLong(1) }
          }
          val length = size ?: resolver.openAssetFileDescriptor(uri,"r")?.use { if(it.length >= 0) it.length else null }
          if(length == null || length < 0) throw Exception("此文件来源无法读取大小，请选择本机文件")
          files.put(JSObject().put("location",uri.toString()).put("name",name ?: "file").put("size",length)
            .put("mime",resolver.getType(uri) ?: "application/octet-stream"))
        }
      }
      invoke.resolve(JSObject().put("files",files))
    } catch(e: Exception) { invoke.reject("无法选择文件：" + e.message) }
  }

  @Command
  fun pickDirectory(invoke: Invoke) {
    try {
      val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).addFlags(
        Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
          Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or Intent.FLAG_GRANT_PREFIX_URI_PERMISSION)
      startActivityForResult(invoke, intent, "directoryResult")
    } catch (e: Exception) { invoke.reject("无法打开目录选择器：${e.message}") }
  }

  @ActivityCallback
  fun directoryResult(invoke: Invoke, result: ActivityResult) {
    try {
      val uri = result.data?.data
      if (result.resultCode != Activity.RESULT_OK || uri == null) { invoke.resolve(JSObject()); return }
      val flags = (result.data?.flags ?: 0) and
        (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
      if (flags and Intent.FLAG_GRANT_WRITE_URI_PERMISSION == 0) throw Exception("未获得写入权限")
      resolver.takePersistableUriPermission(uri, flags)
      invoke.resolve(JSObject().put("directory", uri.toString()))
    } catch (e: Exception) { invoke.reject("无法保留目录授权：${e.message}") }
  }

  private fun displayName(uri: Uri): String? = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
    if (it.moveToFirst()) it.getString(0) else null
  }

  private fun result(name: String, location: String) = JSObject().put("name", name).put("location", location)

  private fun numbered(name: String, number: Int): String {
    if (number == 0) return name
    val dot = name.lastIndexOf('.')
    return if (dot > 0) "${name.substring(0, dot)} ($number)${name.substring(dot)}" else "$name ($number)"
  }

  @Command
  fun openFile(invoke: Invoke) {
    try {
      val location = invoke.parseArgs(ReceiveOptions::class.java).location
      val uri = if (location.startsWith("content://")) Uri.parse(location) else {
        val file = File(location)
        if (!file.isFile) throw Exception("文件已在外部移动或删除，原位置已失效")
        FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
      }
      resolver.openAssetFileDescriptor(uri, "r")?.close() ?: throw Exception("文件已在外部移动或删除，原位置已失效")
      activity.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri, resolver.getType(uri) ?: "application/octet-stream")
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
      invoke.resolve()
    } catch (e: Exception) { invoke.reject("无法打开文件：${e.message}") }
  }

  @Command
  fun openDirectory(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(ReceiveOptions::class.java)
      if (args.directory.startsWith("content://")) {
        val tree = Uri.parse(args.directory)
        val uri = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree))
        activity.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri, DocumentsContract.Document.MIME_TYPE_DIR)
          .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
      } else activity.startActivity(Intent(DownloadManager.ACTION_VIEW_DOWNLOADS))
      invoke.resolve()
    } catch (e: Exception) { invoke.reject("无法打开文件夹，请使用系统文件管理器：${e.message}") }
  }
}
