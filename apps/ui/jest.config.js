const nextJest = require("next/jest");

const createJestConfig = nextJest({
  dir: "./",
});

/** @type {import('jest').Config} */
const customJestConfig = {
  setupFilesAfterEnv: ["<rootDir>/jest.setup.js"],
  testEnvironment: "jest-environment-jsdom",
  moduleNameMapper: {
    "^@/(.*)$": "<rootDir>/src/$1",
  },
  testPathIgnorePatterns: ["<rootDir>/node_modules/", "<rootDir>/.next/"],
  // Transform ES modules from react-markdown and its dependencies
  transformIgnorePatterns: [
    "/node_modules/(?!(react-markdown|remark-gfm|unified|bail|is-plain-obj|trough|vfile|vfile-message|unist-util-stringify-position|mdast-util-from-markdown|mdast-util-to-string|micromark|decode-named-character-reference|character-entities|mdast-util-gfm|mdast-util-gfm-autolink-literal|mdast-util-gfm-footnote|mdast-util-gfm-strikethrough|mdast-util-gfm-table|mdast-util-gfm-task-list-item|micromark-extension-gfm|micromark-extension-gfm-autolink-literal|micromark-extension-gfm-footnote|micromark-extension-gfm-strikethrough|micromark-extension-gfm-table|micromark-extension-gfm-tagfilter|micromark-extension-gfm-task-list-item|ccount|escape-string-regexp|markdown-table|unist-util-visit|unist-util-visit-parents|unist-util-is|space-separated-tokens|comma-separated-tokens|property-information|hast-util-whitespace|remark-parse|remark-rehype|hast-util-to-jsx-runtime|estree-util-is-identifier-name|devlop|hast-util-from-parse5|hastscript|web-namespaces|zwitch|html-url-attributes|mdast-util-to-hast|unist-util-position|trim-lines|micromark-util-.*)/)",
  ],
};

module.exports = createJestConfig(customJestConfig);
