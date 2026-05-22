const fs = require('fs');
const html = fs.readFileSync('/Users/cih1996/work/ai-scaffold/web/static/index.html', 'utf8');

const lines = html.split('\n');
let stack = [];

for (let i = 0; i < lines.length; i++) {
  const line = lines[i];
  
  // A very simple regex to find <div ...> and </div>
  const divOpens = [...line.matchAll(/<div[^>]*>/g)];
  const divCloses = [...line.matchAll(/<\/div>/g)];
  
  for (const open of divOpens) {
    stack.push({ line: i + 1, type: 'div' });
  }
  
  for (const close of divCloses) {
    if (stack.length === 0) {
      console.log(`Extra </div> found at line ${i + 1}`);
    } else {
      stack.pop();
    }
  }
}

if (stack.length > 0) {
  console.log(`Unclosed <div> tags: ${stack.length}`);
  for (const item of stack) {
    console.log(`Unclosed <div ...> at line ${item.line}`);
  }
} else {
  console.log("All <div> tags are balanced!");
}
