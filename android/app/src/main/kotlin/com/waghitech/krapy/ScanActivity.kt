package com.waghitech.krapy

import android.Manifest
import android.app.Activity
import android.app.AlertDialog
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.ImageFormat
import android.graphics.SurfaceTexture
import android.graphics.drawable.GradientDrawable
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.Image
import android.media.ImageReader
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.util.Size
import android.view.Gravity
import android.view.Surface
import android.view.TextureView
import android.view.View
import android.view.ViewGroup
import android.view.WindowInsets
import android.widget.Button
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import kotlin.math.abs
import kotlin.math.max

class ScanActivity : Activity(), TextureView.SurfaceTextureListener {
    private lateinit var texture: TextureView
    private lateinit var status: TextView
    private lateinit var root: FrameLayout
    private var camThread: HandlerThread? = null
    private var camHandler: Handler? = null
    private var decThread: HandlerThread? = null
    private var decHandler: Handler? = null
    private var reader: ImageReader? = null
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var previewSurface: Surface? = null
    @Volatile private var decoding = false
    @Volatile private var done = false
    private val mainHandler = Handler(android.os.Looper.getMainLooper())
    private val basePadH by lazy { dp(16) }
    private val basePadBottom by lazy { dp(12) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setResult(RESULT_CANCELED)
        texture = TextureView(this).apply {
            layoutParams = FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }
        status = TextView(this).apply {
            text = "point at the QR from krapy pair"
            textSize = 14f
            setTextColor(0xFFFFFFFF.toInt())
            setPadding(dp(16), dp(10), dp(16), dp(10))
        }
        val pasteBtn = secondaryButton("paste link instead") { showPasteDialog() }
        val strip = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            background = GradientDrawable().apply { setColor(0x99000000.toInt()) }
            addView(status)
            addView(pasteBtn.apply {
                layoutParams = LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                ).apply {
                    topMargin = dp(4)
                    leftMargin = dp(12)
                    rightMargin = dp(12)
                    bottomMargin = dp(8)
                }
            })
        }
        root = FrameLayout(this).apply {
            addView(texture)
            addView(strip, FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT,
                Gravity.BOTTOM,
            ))
        }
        applyWindowInsets(strip)
        setContentView(root)
        decThread = HandlerThread("qr-decode").also { it.start() }
        decHandler = Handler(decThread!!.looper)
        mainHandler.postDelayed({
            if (!done) {
                status.text =
                    "still looking: hold the phone 20-30 cm away, fill the frame with the QR"
            }
        }, 15_000L)
        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            texture.surfaceTextureListener = this
        } else {
            requestPermissions(arrayOf(Manifest.permission.CAMERA), REQ_PERM)
        }
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        if (requestCode != REQ_PERM) return
        if (grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
            texture.surfaceTextureListener = this
            if (texture.isAvailable) onSurfaceTextureAvailable(texture.surfaceTexture!!, texture.width, texture.height)
        } else {
            Toast.makeText(this, "Camera permission is required to scan pairing QR codes", Toast.LENGTH_LONG).show()
            finish()
        }
    }

    override fun onPause() {
        closeCamera()
        super.onPause()
    }

    override fun onDestroy() {
        mainHandler.removeCallbacksAndMessages(null)
        decThread?.quitSafely()
        decThread = null
        decHandler = null
        super.onDestroy()
    }

    override fun onSurfaceTextureAvailable(surface: SurfaceTexture, width: Int, height: Int) {
        if (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) return
        startCamera(surface)
    }

    override fun onSurfaceTextureSizeChanged(surface: SurfaceTexture, width: Int, height: Int) = Unit
    override fun onSurfaceTextureDestroyed(surface: SurfaceTexture): Boolean {
        closeCamera()
        return true
    }
    override fun onSurfaceTextureUpdated(surface: SurfaceTexture) = Unit

    private fun showPasteDialog() {
        val input = EditText(this).apply {
            setSingleLine()
            val clip = (getSystemService(CLIPBOARD_SERVICE) as ClipboardManager).primaryClip
            val prefilled = clip?.getItemAt(0)?.coerceToText(this@ScanActivity)?.toString()
            if (prefilled != null && prefilled.startsWith("krapy://")) {
                setText(prefilled)
            }
        }
        AlertDialog.Builder(this)
            .setTitle("Paste pairing link")
            .setView(input)
            .setPositiveButton(android.R.string.ok) { _, _ ->
                val text = input.text.toString().trim()
                if (text.isEmpty()) return@setPositiveButton
                deliverUri(text)
            }
            .setNegativeButton(android.R.string.cancel, null)
            .show()
    }

    private fun deliverUri(text: String) {
        if (done) return
        done = true
        setResult(RESULT_OK, Intent().putExtra("uri", text))
        finish()
    }

    private fun startCamera(surface: SurfaceTexture) {
        val mgr = getSystemService(Context.CAMERA_SERVICE) as CameraManager
        val id = mgr.cameraIdList.firstOrNull { cid ->
            mgr.getCameraCharacteristics(cid).get(CameraCharacteristics.LENS_FACING) ==
                CameraCharacteristics.LENS_FACING_BACK
        } ?: run {
            Toast.makeText(this, "no back camera", Toast.LENGTH_SHORT).show()
            finish()
            return
        }
        val chars = mgr.getCameraCharacteristics(id)
        val map = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)!!
        val afModes = chars.get(CameraCharacteristics.CONTROL_AF_AVAILABLE_MODES) ?: intArrayOf()
        val afMode = when {
            CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE in afModes ->
                CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE
            CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO in afModes ->
                CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO
            else -> CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_PICTURE
        }
        val decodeSize = closest(map.getOutputSizes(ImageFormat.YUV_420_888), 1280, 720)
        val previewSize = pickPreview(map.getOutputSizes(SurfaceTexture::class.java), decodeSize)
        surface.setDefaultBufferSize(previewSize.width, previewSize.height)
        camThread = HandlerThread("cam").also { it.start() }
        camHandler = Handler(camThread!!.looper)
        reader = ImageReader.newInstance(
            decodeSize.width,
            decodeSize.height,
            ImageFormat.YUV_420_888,
            2,
        ).apply {
            setOnImageAvailableListener({ r ->
                if (decoding || done) {
                    r.acquireLatestImage()?.close()
                    return@setOnImageAvailableListener
                }
                val image = r.acquireLatestImage() ?: return@setOnImageAvailableListener
                decoding = true
                decHandler?.post {
                    try {
                        scanFrame(image)
                    } catch (_: Throwable) {
                    } finally {
                        image.close()
                        decoding = false
                    }
                }
            }, camHandler)
        }
        try {
            mgr.openCamera(id, object : CameraDevice.StateCallback() {
                override fun onOpened(camera: CameraDevice) {
                    device = camera
                    val preview = Surface(texture.surfaceTexture).also { previewSurface = it }
                    val targets = listOf(preview, reader!!.surface)
                    camera.createCaptureSession(targets, object : CameraCaptureSession.StateCallback() {
                        override fun onConfigured(s: CameraCaptureSession) {
                            session = s
                            val req = camera.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW).apply {
                                addTarget(preview)
                                addTarget(reader!!.surface)
                                set(CaptureRequest.CONTROL_AF_MODE, afMode)
                            }.build()
                            s.setRepeatingRequest(req, null, camHandler)
                        }
                        override fun onConfigureFailed(s: CameraCaptureSession) {
                            Toast.makeText(this@ScanActivity, "camera config failed", Toast.LENGTH_SHORT).show()
                            finish()
                        }
                    }, camHandler)
                }
                override fun onDisconnected(camera: CameraDevice) {
                    camera.close()
                    device = null
                }
                override fun onError(camera: CameraDevice, error: Int) {
                    camera.close()
                    device = null
                    if (!done) finish()
                }
            }, camHandler)
        } catch (_: SecurityException) {
            Toast.makeText(this, "Camera permission is required to scan pairing QR codes", Toast.LENGTH_LONG).show()
            finish()
        }
    }

    private fun scanFrame(image: Image) {
        val y = image.planes[0]
        val w = image.width
        val h = image.height
        val lum = luminance(y.buffer, y.rowStride, y.pixelStride, w, h)
        val text = decode(lum, w, h) ?: return
        if (!text.startsWith("krapy://pair")) {
            runOnUiThread {
                if (!done) status.text = "not a krapy QR"
            }
            return
        }
        try {
            Pairing.parse(text)
        } catch (e: PairingException) {
            runOnUiThread {
                if (!done) status.text = e.message ?: "invalid QR"
            }
            return
        }
        runOnUiThread {
            if (done) return@runOnUiThread
            done = true
            setResult(RESULT_OK, Intent().putExtra("uri", text))
            finish()
        }
    }

    private fun closeCamera() {
        session?.close()
        session = null
        device?.close()
        device = null
        reader?.close()
        reader = null
        previewSurface?.release()
        previewSurface = null
        camThread?.quitSafely()
        camThread = null
        camHandler = null
    }

    private fun closest(sizes: Array<Size>?, tw: Int, th: Int): Size {
        val list = sizes?.toList().orEmpty()
        if (list.isEmpty()) return Size(tw, th)
        return list.minBy { abs(it.width - tw) + abs(it.height - th) }
    }

    private fun pickPreview(sizes: Array<Size>, target: Size): Size {
        val ratio = target.width.toFloat() / target.height
        val near = sizes.filter { abs(it.width.toFloat() / it.height - ratio) < 0.03f }
        val pool = if (near.isNotEmpty()) near else sizes.toList()
        return pool.minBy { it.width * it.height }
    }

    private fun applyWindowInsets(strip: LinearLayout) {
        val padBottom = basePadBottom
        if (Build.VERSION.SDK_INT >= 30) {
            strip.setOnApplyWindowInsetsListener { v, insets ->
                val bars = insets.getInsets(WindowInsets.Type.systemBars())
                val ime = insets.getInsets(WindowInsets.Type.ime())
                val cutout = insets.getInsets(WindowInsets.Type.displayCutout())
                val bottom = max(bars.bottom, ime.bottom)
                v.setPadding(
                    basePadH + max(bars.left, cutout.left),
                    v.paddingTop,
                    basePadH + max(bars.right, cutout.right),
                    padBottom + bottom,
                )
                insets
            }
        } else {
            @Suppress("DEPRECATION")
            strip.setOnApplyWindowInsetsListener { v, insets ->
                val sys = insets.systemWindowInsets
                v.setPadding(
                    basePadH + sys.left,
                    v.paddingTop,
                    basePadH + sys.right,
                    padBottom + sys.bottom,
                )
                insets
            }
        }
        strip.requestApplyInsets()
    }

    private fun dp(value: Int): Int =
        (value * resources.displayMetrics.density + 0.5f).toInt()

    private fun accentColor(): Int = getColor(R.color.accent)

    private fun secondaryButton(label: String, onClick: () -> Unit): Button =
        Button(this).apply {
            text = label
            isAllCaps = false
            minHeight = dp(44)
            setTextColor(accentColor())
            background = GradientDrawable().apply {
                setStroke(dp(1), Color.WHITE)
                cornerRadius = dp(8).toFloat()
                setColor(0)
            }
            stateListAnimator = null
            elevation = 0f
            setOnClickListener { onClick() }
        }

    companion object {
        private const val REQ_PERM = 2
    }
}