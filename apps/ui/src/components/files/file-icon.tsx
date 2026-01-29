import { File, FileText } from "lucide-react";

/** Text file extensions that should use FileText icon */
const TEXT_EXTENSIONS = [
  "txt",
  "md",
  "json",
  "js",
  "ts",
  "tsx",
  "jsx",
  "css",
  "html",
  "py",
  "rs",
  "go",
  "yml",
  "yaml",
  "toml",
  "sh",
  "bash",
  "zsh",
  "c",
  "cpp",
  "h",
  "hpp",
  "java",
  "rb",
  "php",
  "sql",
  "xml",
  "csv",
];

export interface FileIconProps {
  /** File extension (without the dot) */
  extension: string;
  /** Icon size class (default: "h-4 w-4") */
  className?: string;
}

/**
 * File icon component that shows different icons based on file extension.
 * Uses FileText for text/code files, generic File for others.
 */
export function FileIcon({ extension, className = "h-4 w-4" }: FileIconProps) {
  const isText = TEXT_EXTENSIONS.includes(extension.toLowerCase());

  if (isText) {
    return <FileText className={`${className} text-gray-500`} />;
  }

  return <File className={`${className} text-gray-400`} />;
}
