package domica.caprust.editor;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.view.ViewGroup;

public class StartActivity extends Activity {
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_start);
        Button btnNew = findViewById(R.id.btn_new);
        Button btnOpen = findViewById(R.id.btn_open);
        btnNew.setOnClickListener(v -> {
            startActivity(new Intent(this, EditorActivity.class));
        });
        btnOpen.setOnClickListener(v -> {
            Intent pick = new Intent(Intent.ACTION_GET_CONTENT);
            pick.setType("*/*");
            startActivityForResult(pick, 41);
        });
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == 41 && resultCode == RESULT_OK && data != null) {
            startActivity(new Intent(this, EditorActivity.class));
        }
    }
}
