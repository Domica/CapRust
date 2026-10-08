with open('crates/ui/src/timeline/mod.rs', 'r', encoding='utf-8') as f:
    content = f.read()

lines = content.split('\n')

# Find the block to replace
start = None
end = None
for i, line in enumerate(lines):
    if 'Also verify center line exists (0 dB reference) - it\'s drawn with semi-transparent white' in line:
        start = i
    if start is not None and 'assert!(center_line.is_some(), "center line (0 dB) should exist");' in line:
        end = i
        break

if start is not None and end is not None:
    new_lines = [
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
    new_content = '\n'.join(lines[:591] + new_lines + lines[608:])
    with open('crates/ui/src/timeline/mod.rs', 'w', encoding='utf-8') as f:
        f.write('\n'.join(new_content))
    print('Done')
else:
    print('Could not find the block to replace')