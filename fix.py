import re

with open('/Users/cih1996/work/ai-scaffold/web/static/index.html', 'r', encoding='utf-8') as f:
    content = f.read()

# Fix flex-shrink on cards
content = content.replace(
    '.raw-card { background: var(--bg);',
    '.raw-card { flex-shrink: 0; background: var(--bg);'
)
content = content.replace(
    '.config-section, .tool-card { background: var(--bg);',
    '.config-section, .tool-card { flex-shrink: 0; background: var(--bg);'
)

# Swap order of .left and .right in .main
left_match = re.search(r'(<!-- Left: Chat Centered -->.*?)(?=<!-- Right: Events & Tools -->)', content, re.DOTALL)
right_match = re.search(r'(<!-- Right: Events & Tools -->.*?)(?=\s*</div>\s*</div>\s*</div>\s*<script>)', content, re.DOTALL)

if left_match and right_match:
    left_str = left_match.group(1)
    right_str = right_match.group(1)
    
    # Replace in content
    new_main_content = right_str + 'import re