package domica.caprust.editor

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.widget.Button

class StartActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_start)
        val btnNew = findViewById<Button>(R.id.btn_new)
        val btnOpen = findViewById<Button>(R.id.btn_open)
        btnNew.setOnClickListener {
            startActivity(Intent(this, EditorActivity::class.java))
        }
        btnOpen.setOnClickListener {
            val pick = Intent(Intent.ACTION_GET_CONTENT).apply { type = "*/*" }
            startActivityForResult(pick, 41)
        }
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == 41 && resultCode == RESULT_OK && data != null) {
            startActivity(Intent(this, EditorActivity::class.java))
        }
    }
}
