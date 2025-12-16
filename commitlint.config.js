// Conventional Commits configuration
// See: https://www.conventionalcommits.org
// See: https://commitlint.js.org

module.exports = {
  extends: ['@commitlint/config-conventional'],
  rules: {
    // Type must be one of the allowed values
    'type-enum': [
      2,
      'always',
      [
        'feat',     // New feature
        'fix',      // Bug fix
        'docs',     // Documentation changes
        'style',    // Code style (formatting, semicolons, etc.)
        'refactor', // Code refactoring without feature/fix
        'perf',     // Performance improvements
        'test',     // Adding or updating tests
        'chore',    // Build process, dependencies, tooling
        'ci',       // CI configuration changes
      ],
    ],
    // Subject must not be empty
    'subject-empty': [2, 'never'],
    // Type must not be empty
    'type-empty': [2, 'never'],
    // Subject must be lowercase
    'subject-case': [2, 'always', 'lower-case'],
    // No period at end of subject
    'subject-full-stop': [2, 'never', '.'],
    // Body max line length
    'body-max-line-length': [1, 'always', 100],
  },
};
