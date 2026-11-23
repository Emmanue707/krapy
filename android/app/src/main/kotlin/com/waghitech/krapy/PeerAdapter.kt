package com.waghitech.krapy

import android.content.Context
import android.graphics.Typeface
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.BaseAdapter
import android.widget.LinearLayout
import android.widget.TextView

class PeerAdapter(
    private val context: Context,
    private var rows: List<Peer>,
    private val primary: Int,
    private val secondary: Int,
) : BaseAdapter() {

    fun update(next: List<Peer>) {
        rows = next
        notifyDataSetChanged()
    }

    override fun getCount(): Int = rows.size
    override fun getItem(position: Int): Peer = rows[position]
    override fun getItemId(position: Int): Long = position.toLong()

    override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
        val row = convertView as? LinearLayout ?: newRow()
        val peer = rows[position]
        (row.getChildAt(0) as TextView).text = peer.name
        (row.getChildAt(1) as TextView).text = "${peer.host}:${peer.port}"
        return row
    }

    private fun newRow(): LinearLayout {
        val pad = (12 * context.resources.displayMetrics.density).toInt()
        return LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_VERTICAL
            minimumHeight = pad * 4
            setPadding(pad, pad / 2, pad, pad / 2)
            addView(TextView(context).apply {
                textSize = 16f
                setTypeface(typeface, Typeface.BOLD)
                setTextColor(primary)
            })
            addView(TextView(context).apply {
                textSize = 13f
                setTextColor(secondary)
            })
        }
    }
}
