with open('crates/media-io/src/export_graph.rs', 'r', encoding='utf-8') as f:
    content = f.read()
old = '[a{i}_s][a{i}_blur][{a_label}]maskedmerge[{an_out}];'
new = '[a{i}_s][a{i}_blur]blend=all_mode=normal[{an_out}];'
content = content.replace(old, new)
with open('crates/media-io/src/export_graph.rs', 'w', encoding='utf-8') as f:
    f.write(content)
print('done')
