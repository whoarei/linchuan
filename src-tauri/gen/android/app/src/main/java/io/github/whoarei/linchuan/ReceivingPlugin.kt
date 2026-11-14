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

  @Command
  fun saveFile(invoke: Invoke) {
    val args = invoke.parseArgs(ReceiveOptions::class.java)
    if (args.directory == "@downloads" && Build.VERSION.SDK_INT < 29 &&
      activity.checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) != PackageManager.PERMISSION_GRANTED) {
      requestPermissionForAlias("legacy-storage", invoke, "storagePermissionResult")
      return
    }
    Thread {
      try {
        val result = if (args.directory != "@downloads") saveDocument(args)
          else if (Build.VERSION.SDK_INT >= 29) saveDownload(args) else saveLegacy(args)
        invoke.resolve(result)
      } catch (e: Exception) { invoke.reject("保存失败，请检查目录权限和剩余空间：${e.message}") }
    }.start()
  }

  @PermissionCallback
  fun storagePermissionResult(invoke: Invoke) {
    if (activity.checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE) == PackageManager.PERMISSION_GRANTED) saveFile(invoke)
    else invoke.reject("未获得存储权限，文件仍保留在应用内，可在授权后重试或更改接收目录")
  }

  private fun displayName(uri: Uri): String? = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
    if (it.moveToFirst()) it.getString(0) else null
  }

  private fun result(name: String, location: String) = JSObject().put("name", name).put("location", location)

  @android.annotation.TargetApi(29)
  private fun saveDownload(args: ReceiveOptions): JSObject {
    val values = ContentValues().apply {
      put(MediaStore.Downloads.DISPLAY_NAME, args.name)
      put(MediaStore.Downloads.MIME_TYPE, args.mime)
      put(MediaStore.Downloads.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/邻传")
      put(MediaStore.Downloads.IS_PENDING, 1)
    }
    val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: throw Exception("无法创建下载文件")
    try {
      copyToUri(args.sourcePath, uri)
      val done = ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }
      if (resolver.update(uri, done, null, null) != 1) throw Exception("无法完成文件保存")
      return result(displayName(uri) ?: args.name, uri.toString())
    } catch (e: Exception) { resolver.delete(uri, null, null); throw e }
  }

  private fun saveDocument(args: ReceiveOptions): JSObject {
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
    try {
      copyToUri(args.sourcePath, uri)
      return result(displayName(uri) ?: name, uri.toString())
    } catch (e: Exception) { DocumentsContract.deleteDocument(resolver, uri); throw e }
  }

  private fun copyToUri(source: String, uri: Uri) {
    File(source).inputStream().use { input ->
      (resolver.openOutputStream(uri, "w") ?: throw Exception("目录不可写")).use { output -> input.copyTo(output) }
    }
  }

  @Suppress("DEPRECATION")
  private fun saveLegacy(args: ReceiveOptions): JSObject {
    val folder = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "邻传")
    if (!folder.isDirectory && !folder.mkdirs()) throw Exception("无法创建下载目录")
    val target = (0..9999).asSequence().map { File(folder, numbered(args.name, it)) }.firstOrNull { it.createNewFile() }
      ?: throw Exception("同名文件过多")
    try {
      File(args.sourcePath).inputStream().use { input -> target.outputStream().use { output ->
        input.copyTo(output)
        output.fd.sync()
      } }
      MediaScannerConnection.scanFile(activity, arrayOf(target.absolutePath), arrayOf(args.mime), null)
      return result(target.name, target.absolutePath)
    } catch (e: Exception) { target.delete(); throw e }
  }

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
        if (!file.isFile) throw Exception("文件已移动或删除，可重新保存")
        FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", file)
      }
      resolver.openAssetFileDescriptor(uri, "r")?.close() ?: throw Exception("文件已移动或删除，可重新保存")
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
