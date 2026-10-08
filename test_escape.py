path = r'C:\Users\domag\Pictures\CapRust\caprust-frame-12-34-56-789.png'
import re

s = path
if ' ' in s or '&' in s or '^' in s or '%' in s:
    result = '"' + s.replace('"', '\\"') + '"'
else:
    result = s
print("Result:", result)
print("Length:", len(result))