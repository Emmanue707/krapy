package com.waghitech.krapy

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.ImageFormat
import android.graphics.SurfaceTexture
import android.view.Surface
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.media.Image
import android.media.ImageReader
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.util.Size
import android.view.TextureView
import android.view.ViewGroup
import android.widget.Toast
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.NotFoundException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import com.google.zxing.PlanarYUVLuminanceSource
import kotlin.math.abs

class ScanActivity : Activity(), TextureView.SurfaceTextureListener {
    private lateinit var texture: TextureView
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
    private val qr = QRCodeReader()
    private val hints = mapOf(
        DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
        DecodeHintType.TRY_HARDER to false,
    )

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setResult(RESULT_CANCELED)
        texture = TextureView(this).apply {
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            )
        }
        setContentView(texture)
        decThread = HandlerThread("qr-decode").also { it.start() }
        decHandler = Handler(decThread!!.looper)
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
        val map = mgr.getCameraCharacteristics(id).get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)!!
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
        val buf = y.buffer
        val row = y.rowStride
        val w = image.width
        val h = image.height
        val data = ByteArray(row * h)
        buf.position(0)
        buf.get(data)
        val src = PlanarYUVLuminanceSource(data, row, h, 0, 0, w, h, false)
        val text = try {
            qr.decode(BinaryBitmap(HybridBinarizer(src)), hints).text
        } catch (_: NotFoundException) {
            return
        } catch (_: Exception) {
            return
        }
        if (!text.startsWith("krapy://pair")) return
        try {
            Pairing.parse(text)
        } catch (_: PairingException) {
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

    companion object {
        private const val REQ_PERM = 2
    }
}