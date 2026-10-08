with open('crates/ui/src/timeline/mod.rs', 'r', encoding='utf-8') as f:
    content = f.read()

lines = content.split('\n')

new_block = [
    '        // Also verify center line exists (0 dB reference) - it\'s a horizontal line at center_y',
    '        let center_y = full.bottom() - db_to_normalized_y(0.0).clamp(0.0, 1.0) * full.height();',
    '        let center_line = shapes.iter().find_map(|s| match &s.shape {',
    '            egui::Shape::Path(l) => {',
    '                let is_horizontal = l.points.len() >= 2',
    '                    && (l.points.first().unwrap().y - center_y).abs() < 1.0',
    '                    && (l.points.last().unwrap().y - center_y).abs() < 1.0;',
    '                if is_horizontal {',
    '                    Some(l.points.clone())',
    '                } else {',
    '                    None',
    '                }',
    '            }',
    '            egui::Shape::Line(line) => {',
    '                // Center line is drawn as a Line shape, not Path',
    '                let is_horizontal = (line.points[0].y - center_y).abs() < 1.0',
    '                    && (line.points[1].y - center_y).abs() < 1.0;',
    '                if is_horizontal {',
    '                    Some(line.points.to_vec())',
    '                } else {',
    '                    None',
    '                }',
    '            }',
    '            _ => None,',
    '        });',
    '        assert!(center_line.is_some(), "center line (0 dB) should exist");',
    '    }'
]

# Lines are 0-indexed. The block to replace is lines 591-607 (1-indexed: 592-608)
# Python list is 0-indexed, so lines[591] = line 592 (1-indexed)
# We want to replace lines[591:608] (591 inclusive, 608 exclusive)
new_content = lines[:591] + new_block + lines[608:]

with open('crates/ui/src/timeline/mod.rs', 'w', encoding='utf-8') as f:
    f.write('\n'.join(new_content))
print('Done')