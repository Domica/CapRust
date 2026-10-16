import io

path = 'F:/Dev/projects/CapRust/crates/ui/src/app.rs'
with open(path, 'r', encoding='utf-8', newline='') as f:
    content = f.read()

# Remove the two empty lines after the function closing brace
old = "}\n    \n    \n    /// Settings for something this small: S fits the space under the"
new = "}\n    /// Settings for something this small: S fits the space under the"

if old in content:
    content = content.replace(old, new, 1)
    with open(path, 'w', encoding='utf-8', newline='') as f:
        f.write(content)
    print("Removed empty lines")
else:
    print("Pattern not found, trying alt...")
    old2 = "}\n    }\n    \n    ///"
    new2 = "}\n    ///"
    if old2 in content:
        content = content.replace(old2, new2, 1)
        with open(path, 'w', encoding='utf-8', newline='') as f:
            f.write(content)
        print("Removed empty lines alt")
    else:
        print("Could not find pattern to remove")
        # Show surrounding lines
        idx = content.find("fn timeline_zone_split_ratio")
        if idx != -1:
            print("Function found at", idx)
            print(content[idx-100:idx+100])