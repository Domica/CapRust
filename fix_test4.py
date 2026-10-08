with open('crates/ui/src/timeline/mod.rs', 'r', encoding='utf-8') as f:
    content = f.read()

old = """        // Also verify center line exists (0 dB reference)
        let center_line = shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Path(l) => {
                // Check if this is a horizontal line at center_y
                let is_horizontal = l.points.len() >= 2
                    && (l.points.first().unwrap().y - center_y).abs() < 1.0
                    && (l.points.last().unwrap().y - center_y).abs() < 1.0;
                if is_horizontal {
                    Some(l.points.clone())
                } else {
                    None
                }
            }
            _ => None,
        });
        assert!(center_line.is_some(), "center line (0 dB) should exist");
    }"""

new = """        // Also verify center line exists (0 dB reference)
        let center_y = full.bottom() - db_to_normalized_y(0.0).clamp(0.0, 1.0) * full.height();
        let center_line = shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Path(l) => {
                // Check if this is a horizontal line at center_y
                let is_horizontal = l.points.len() >= 2
                    && (l.points.first().unwrap().y - center_y).abs() < 1.0
                    && (l.points.last().unwrap().y - center_y).abs() < 1.0;
                if is_horizontal {
                    Some(l.points.clone())
                } else {
                    None
                }
            }
            _ => None,
        });
        assert!(center_line.is_some(), "center line (0 dB) should exist");
    }"""

content = content.replace(old, new)
with open('crates/ui/src/timeline/mod.rs', 'w', encoding='utf-8') as f:
    f.write(content)
print('Done')