import io

path = 'F:/Dev/projects/CapRust/crates/ui/src/app.rs'
with open(path, 'r', encoding='utf-8', newline='') as f:
    content = f.read()

start_marker = "    /// Return the top/bottom split ratio for zones based on timeline size."
start_idx = content.find(start_marker)

if start_idx == -1:
    print("Could not find function start")
else:
    func_end_marker = "    }"
    func_end_idx = content.find(func_end_marker, start_idx)
    
    if func_end_idx == -1:
        print("Could not find function end")
    else:
        show_marker = "    /// Settings for something this small: S fits the space under the"
        show_idx = content.find(show_marker, func_end_idx)
        
        if show_idx == -1:
            print("Could not find show_timeline doc")
        else:
            # Remove from start_idx to show_idx (exclusive), keeping show_timeline doc
            new_content = content[:start_idx] + content[show_idx:]
            
            with open(path, 'w', encoding='utf-8', newline='') as f:
                f.write(new_content)
            print("Removed function block, new content length:", len(new_content))