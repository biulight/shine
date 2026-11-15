// Extend the loaded grammars in place so shell aliases and nested grammars retain
// their references. Run after strings/comments, but before generic commands.
module.exports = function includeShine(Prism) {
  const patterns = {
    bash: /(?<=(?:(?<!\\\r?\n)^|(?<!\\)[;|&])[ \t]*(?:\([ \t]*)?(?:(?:sudo|command|exec)[ \t]+)?(?:[A-Za-z_]\w*=[\w./:-]+[ \t]+)*)shine(?=$|[\s;|&)])/m,
    powershell: /(?<=(?:(?<!`\r?\n)^|(?<!`)[;|&({])[ \t]*(?:\$[\w:]+[ \t]*=[ \t]*)?(?:sudo[ \t]+)?)shine(?:\.exe)?(?=$|[\s;|&)}])/im,
  };

  for (const [language, pattern] of Object.entries(patterns)) {
    const grammar = Prism.languages[language];
    const entries = Object.entries(grammar);
    for (const key of Object.keys(grammar)) delete grammar[key];
    for (const [key, value] of entries) {
      if (key === 'function') {
        grammar['shine-command'] = {
          pattern,
          // Match against the original source, not a fragment after a string
          // token: otherwise `echo "text" shine` could look like a command start.
          greedy: true,
        };
      }
      grammar[key] = value;
    }
  }
};
