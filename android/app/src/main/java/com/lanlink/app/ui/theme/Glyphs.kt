package com.lanlink.app.ui.theme

import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp
import androidx.compose.ui.graphics.Color

/** Line icons drawn for LANLink (the core Material set lacks these). */
object Glyphs {
    private fun icon(name: String, block: androidx.compose.ui.graphics.vector.PathBuilder.() -> Unit): ImageVector =
        ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).path(
            stroke = SolidColor(Color.Black), strokeLineWidth = 1.8f,
            strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round,
            pathBuilder = block,
        ).build()

    val Menu = icon("menu") {
        moveTo(5f, 7f); lineTo(19f, 7f); moveTo(5f, 12f); lineTo(19f, 12f); moveTo(5f, 17f); lineTo(19f, 17f)
    }
    val Close = icon("close") {
        moveTo(6.5f, 6.5f); lineTo(17.5f, 17.5f); moveTo(17.5f, 6.5f); lineTo(6.5f, 17.5f)
    }
    val Laptop = icon("laptop") {
        moveTo(5f, 6f); lineTo(19f, 6f); lineTo(19f, 15f); lineTo(5f, 15f); close()
        moveTo(3f, 18.5f); lineTo(21f, 18.5f)
    }
    val Keyboard = icon("keyboard") {
        moveTo(3f, 7f); lineTo(21f, 7f); lineTo(21f, 17f); lineTo(3f, 17f); close()
        moveTo(6.5f, 10.5f); lineTo(7f, 10.5f); moveTo(10f, 10.5f); lineTo(10.5f, 10.5f)
        moveTo(13.5f, 10.5f); lineTo(14f, 10.5f); moveTo(17f, 10.5f); lineTo(17.5f, 10.5f)
        moveTo(8f, 14f); lineTo(16f, 14f)
    }
    val Mouse = icon("mouse") {
        moveTo(12f, 3.5f); curveTo(8.5f, 3.5f, 7f, 6f, 7f, 9f); lineTo(7f, 15f)
        curveTo(7f, 18.5f, 9.5f, 20.5f, 12f, 20.5f); curveTo(14.5f, 20.5f, 17f, 18.5f, 17f, 15f)
        lineTo(17f, 9f); curveTo(17f, 6f, 15.5f, 3.5f, 12f, 3.5f); close()
        moveTo(12f, 6.5f); lineTo(12f, 9.5f)
    }
    val Touch = icon("touch") {
        moveTo(9f, 11f); lineTo(9f, 5.5f); curveTo(9f, 4.5f, 10.5f, 4.5f, 10.5f, 5.5f); lineTo(10.5f, 11f)
        moveTo(10.5f, 10f); curveTo(10.5f, 9f, 12f, 9f, 12f, 10f); lineTo(12f, 11.5f)
        moveTo(12f, 10.5f); curveTo(12f, 9.5f, 13.5f, 9.5f, 13.5f, 10.5f); lineTo(13.5f, 12f)
        moveTo(13.5f, 11f); curveTo(13.5f, 10f, 15f, 10f, 15f, 11f); lineTo(15f, 15f)
        curveTo(15f, 18f, 13f, 20f, 11f, 20f); curveTo(9f, 20f, 8f, 19f, 7f, 17f); lineTo(5.5f, 14f)
        curveTo(5f, 13f, 6.5f, 12f, 7.5f, 13f); lineTo(9f, 14.5f)
    }
    val Monitor = icon("monitor") {
        moveTo(3.5f, 5f); lineTo(20.5f, 5f); lineTo(20.5f, 16f); lineTo(3.5f, 16f); close()
        moveTo(9f, 20f); lineTo(15f, 20f); moveTo(12f, 16f); lineTo(12f, 20f)
    }
    val Gauge = icon("gauge") {
        moveTo(4f, 16f); curveTo(4f, 11f, 7.5f, 7f, 12f, 7f); curveTo(16.5f, 7f, 20f, 11f, 20f, 16f)
        moveTo(12f, 16f); lineTo(15.5f, 11f)
    }
    val Quality = icon("quality") {
        moveTo(4f, 18f); lineTo(4f, 14f); moveTo(9.3f, 18f); lineTo(9.3f, 10f)
        moveTo(14.6f, 18f); lineTo(14.6f, 7f); moveTo(20f, 18f); lineTo(20f, 4f)
    }
    val Power = icon("power") {
        moveTo(12f, 3.5f); lineTo(12f, 11f)
        moveTo(7.5f, 6.5f); curveTo(5.5f, 8f, 4.5f, 10f, 4.5f, 12.5f); curveTo(4.5f, 16.6f, 7.9f, 20f, 12f, 20f)
        curveTo(16.1f, 20f, 19.5f, 16.6f, 19.5f, 12.5f); curveTo(19.5f, 10f, 18.5f, 8f, 16.5f, 6.5f)
    }
    val Zoom = icon("zoom") {
        moveTo(10.5f, 4.5f); curveTo(13.8f, 4.5f, 16.5f, 7.2f, 16.5f, 10.5f); curveTo(16.5f, 13.8f, 13.8f, 16.5f, 10.5f, 16.5f)
        curveTo(7.2f, 16.5f, 4.5f, 13.8f, 4.5f, 10.5f); curveTo(4.5f, 7.2f, 7.2f, 4.5f, 10.5f, 4.5f); close()
        moveTo(15f, 15f); lineTo(20f, 20f); moveTo(8f, 10.5f); lineTo(13f, 10.5f)
    }
    val Shield = icon("shield") {
        moveTo(12f, 3.5f); lineTo(19f, 6f); lineTo(19f, 11.5f); curveTo(19f, 15.5f, 16f, 19f, 12f, 20.5f)
        curveTo(8f, 19f, 5f, 15.5f, 5f, 11.5f); lineTo(5f, 6f); close()
    }
    val QrScan = icon("qr") {
        moveTo(4f, 9f); lineTo(4f, 4f); lineTo(9f, 4f); moveTo(15f, 4f); lineTo(20f, 4f); lineTo(20f, 9f)
        moveTo(20f, 15f); lineTo(20f, 20f); lineTo(15f, 20f); moveTo(9f, 20f); lineTo(4f, 20f); lineTo(4f, 15f)
        moveTo(8f, 12f); lineTo(16f, 12f)
    }
}
