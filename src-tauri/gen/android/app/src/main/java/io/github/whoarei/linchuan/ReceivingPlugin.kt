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
  var sourcePath: String = ""
  var name: String = "file"
  var mime: String = "application/octet-stream"
  var location: String = ""
}

@TauriPlugin(permissions = [Permission(alias = "legacy-storage", strings = [Manifest.permission.WRITE_EXTERNAL_STORAGE])])
class ReceivingPlugin(private val activity: Activity) : Plugin(activity) {
  private val resolver get() = activity.contentResolver

  @Command
  fun storageInfo(invoke: Invoke) {
    try {
      val directory = invoke.parseArgs(ReceiveOptions::class.java).directory
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
      var existing = target
      while (!existing.isDirectory) existing = existing.parentFile ?: throw Exception("目标磁盘不可用")
      // Emulated shared storage and private app storage consume the same /data volume.
      val physical = if (Environment.isExternalStorageEmulated(root)) Environment.getDataDirectory() else existing
      invoke.resolve(JSObject().put("volume", "unix:${Os.stat(physical.absolutePath).st_dev}")
        .put("available", StatFs(existing.absolutePath).availableBytes).put("path", existing.absolutePath))
    } catch (e: Exception) { invoke.reject("无法检查目标剩余空间：${e.message}") }
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
        if (args.directory == "@downloads" && Build.VERSION.SDK_INT >= 29) {
          val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, args.name)
            put(MediaStore.Downloads.MIME_TYPE, args.mime)
            put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/邻传")
            put(MediaStore.Downloads.IS_PENDING, 1)
          }
          val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: throw Exception("无法创建文件")
          location = uri.toString()
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
          val name = (0..9999).asSequence().map { numbered(args.name, it) }.firstOrNull { it !in names }
            ?: throw Exception("同名文件过多")
          val uri = DocumentsContract.createDocument(resolver, parent, args.mime, name) ?: throw Exception("无法创建文件")
          location = uri.toString()
          descriptor = resolver.openFileDescriptor(uri, "w") ?: throw Exception("目标目录不可写")
        } else {
          val folder = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "邻传")
          if (!folder.isDirectory && !folder.mkdirs()) throw Exception("无法创建下载目录")
          val target = (0..9999).asSequence().map { File(folder, numbered(args.name, it)) }.firstOrNull { it.createNewFile() }
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
      val location = invoke.parseArgs(ReceiveOptions::class.java).location
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
