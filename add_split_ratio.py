import io

path = 'F:/Dev/projects/CapRust/crates/ui/src/app.rs'
with open(path, 'r', encoding='utf-8', newline='') as f:
    content = f.read()

# Find the position - after the closing brace of timeline_height_min
# Look for the pattern: "}    /// The picker lives"
pos = content.find("}    /// The picker lives on the timeline")
if pos == -1:
    print("Pattern not found, searching alternative...")
    # Try alternative pattern
    pos = content.find("/// The picker lives on the timeline")
    if pos == -1:
        print("Alternative pattern not found either")
    else:
        # Insert after found position + newline
        insert_pos = pos + content[pos:].index('\n')
        new_function = '''

    /// Return the top/bottom split ratio for zones based on timeline size.
    /// S: zones expand to fill vertical space (0.7 top, 0.3 bottom).
    /// M/L: zones use default 50/50 split.
    fn timeline_zone_split_ratio(&self) -> f32 {
        match self.settings.timeline_size {
            caprust_core::TimelineSize::S => 0.7,
            caprust_core::TimelineSize::M | caprust_core::TimelineSize::L => 0.5,
        }
    }

'''
        new_content = content[:insert_pos] + new_function + content[insert_pos:]
        with open(path, 'w', encoding='utf-8', newline='') as f:
            f.write(new_content)
        print("Inserted successfully at", insert_pos)
else:
    # Insert after the found position + the newline that follows
    insert_pos = pos + len("}    /// The picker lives on the timeline")
    new_function = '''

    /// Return the top/bottom split ratio for zones based on timeline size.
    /// S: zones expand to fill vertical space (0.7 top, 0.3 bottom).
    /// M/L: zones use default 50/50 split.
    fn timeline_zone_split_ratio(&self) -> f32 {
        match self.settings.timeline_size {
            caprust_core::TimelineSize::S => 0.7,
            caprust_core::TimelineSize::M | caprust_core::TimelineSize::L => 0.5,
        }
    }

'''
    new_content = content[:insert_pos] + new_function + content[insert_pos:]
    with open(path, 'w', encoding='utf-8', newline='') as f:
        f.write(new_content)
    print("Inserted successfully at", insert_pos)