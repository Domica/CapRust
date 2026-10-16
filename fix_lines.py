import io

path = 'F:/Dev/projects/CapRust/crates/ui/src/app.rs'
with open(path, 'r', encoding='utf-8', newline='') as f:
    content = f.read()

# Remove the two empty lines after the function closing brace
# The pattern is: "}\n    \n    \n    ///" but we need to be precise
# Let's find and replace the specific section

# Look for "    }\n    \n    \n    ///" and replace with "    }\n    ///"
old = "    }\n    \n    \n    /// Settings for something this small: S fits the space under the"
new = "    }\n    /// Settings for something this small: S fits the space under the"

if old in content:
    content = content.replace(old, new, 1)
    with open(path, 'w', encoding='utf-8', newline='') as f:
        f.write(content)
    print("Fixed empty lines")
else:
    print("Pattern not found, showing context...")
    # Try to find the function closing
    idx = content.find("fn timeline_zone_split_ratio")
    if idx != -1:
        print("Function at", idx)
        print(content[idx:idx+300])