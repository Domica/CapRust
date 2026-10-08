with open('Cargo.toml', 'r') as f:
    content = f.read()
content = content.replace('version = "0.9.10"', 'version = "0.9.11"')
with open('Cargo.toml', 'w') as f:
    f.write(content)
print('Done')