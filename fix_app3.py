with open('crates/ui/src/app.rs', 'r', encoding='utf-8') as f:
    content = f.read()

old = '''                            // Check if object removal is enabled - check pending edits first (checkbox updates go through pending)
                            let is_enabled = self.properties.pending.iter().rev().find_map(|edit| {
                                match edit {
                                    crate::panels::clip_properties::PendingEdit::SetObjectRemovalEnabled(enabled) => Some(*enabled),
                                    _ => None,
                                }
                            }).unwrap_or(clip.object_removal_enabled);
                            
                            if is_enabled {'''

new = '''                            // Check if object removal is enabled - check pending edits first (checkbox updates go through pending)
                            // Also check clip field as fallback (in case pending edit was already processed)
                            let is_enabled = self.properties.pending.iter().rev().find_map(|edit| {
                                match edit {
                                    crate::panels::clip_properties::PendingEdit::SetObjectRemovalEnabled(enabled) => Some(*enabled),
                                    _ => None,
                                }
                            }).unwrap_or(clip.object_removal_enabled);
                            
                            // Also check clip field directly as fallback
                            let is_enabled = is_enabled || clip.object_removal_enabled;

                            if is_enabled {'''

if old in content:
    content = content.replace(old, new)
    with open('crates/ui/src/app.rs', 'w', encoding='utf-8') as f:
        f.write(content)
    print('Replaced successfully')
else:
    print('Old text not found')
    idx = content.find('Check if object removal is enabled')
    if idx >= 0:
        print(repr(content[idx:idx+500]))