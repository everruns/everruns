"use client";

/**
 * FileTree Component (AI Elements)
 *
 * Styled for Slate Design System:
 * - Sharp corners (0px radius)
 * - Grayscale dominant with accent colors
 * - Compact spacing for dense file trees
 */

import type { HTMLAttributes, ReactNode } from "react";

import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import { ChevronRightIcon, FileIcon, FolderIcon, FolderOpenIcon } from "lucide-react";
import { createContext, useCallback, useContext, useState } from "react";

interface FileTreeContextType {
  expandedPaths: Set<string>;
  togglePath: (path: string) => void;
  selectedPath?: string;
  onSelect?: (path: string) => void;
}

// Default noop for context default value
// oxlint-disable-next-line eslint(no-empty-function)
const noop = () => {};

const FileTreeContext = createContext<FileTreeContextType>({
  // oxlint-disable-next-line eslint-plugin-unicorn(no-new-builtin)
  expandedPaths: new Set(),
  togglePath: noop,
});

export type FileTreeProps = Omit<HTMLAttributes<HTMLDivElement>, "onSelect"> & {
  expanded?: Set<string>;
  defaultExpanded?: Set<string>;
  selectedPath?: string;
  onSelect?: (path: string) => void;
  onExpandedChange?: (expanded: Set<string>) => void;
};

export const FileTree = ({
  expanded: controlledExpanded,
  defaultExpanded = new Set(),
  selectedPath,
  onSelect,
  onExpandedChange,
  className,
  children,
  ...props
}: FileTreeProps) => {
  const [internalExpanded, setInternalExpanded] = useState(defaultExpanded);
  const expandedPaths = controlledExpanded ?? internalExpanded;

  const togglePath = (path: string) => {
    const newExpanded = new Set(expandedPaths);
    if (newExpanded.has(path)) {
      newExpanded.delete(path);
    } else {
      newExpanded.add(path);
    }
    setInternalExpanded(newExpanded);
    onExpandedChange?.(newExpanded);
  };

  return (
    <FileTreeContext.Provider value={{ expandedPaths, onSelect, selectedPath, togglePath }}>
      <div
        className={cn("bg-background font-mono text-sm", className)}
        role="tree"
        aria-label="File tree"
        {...props}
      >
        <div className="py-1">{children}</div>
      </div>
    </FileTreeContext.Provider>
  );
};

interface FileTreeFolderContextType {
  path: string;
  name: string;
  isExpanded: boolean;
}

const FileTreeFolderContext = createContext<FileTreeFolderContextType>({
  isExpanded: false,
  name: "",
  path: "",
});

export type FileTreeFolderProps = HTMLAttributes<HTMLDivElement> & {
  path: string;
  name: string;
  /** Actions rendered inline with the folder name (e.g. FileTreeActions) */
  actions?: ReactNode;
};

export const FileTreeFolder = ({
  path,
  name,
  actions,
  className,
  children,
  ...props
}: FileTreeFolderProps) => {
  const { expandedPaths, togglePath, selectedPath, onSelect } = useContext(FileTreeContext);
  const isExpanded = expandedPaths.has(path);
  const isSelected = selectedPath === path;

  const handleOpenChange = useCallback(() => {
    togglePath(path);
  }, [togglePath, path]);

  const handleSelect = useCallback(() => {
    onSelect?.(path);
  }, [onSelect, path]);

  return (
    <FileTreeFolderContext.Provider value={{ isExpanded, name, path }}>
      <Collapsible onOpenChange={handleOpenChange} open={isExpanded}>
        <div className={cn("group", className)} role="treeitem" {...props}>
          <CollapsibleTrigger asChild>
            <button
              className={cn(
                "flex w-full items-center gap-1.5 px-2 py-1 text-left transition-colors",
                "hover:bg-muted/60",
                isSelected && "bg-accent/20 text-accent-foreground",
              )}
              onClick={handleSelect}
              type="button"
            >
              <ChevronRightIcon
                className={cn(
                  "size-3.5 shrink-0 text-muted-foreground/70 transition-transform duration-150",
                  isExpanded && "rotate-90",
                )}
              />
              <FileTreeIcon>
                {isExpanded ? (
                  <FolderOpenIcon className="size-4 text-amber-500" />
                ) : (
                  <FolderIcon className="size-4 text-amber-500/80" />
                )}
              </FileTreeIcon>
              <FileTreeName className="font-medium">{name}</FileTreeName>
              {actions}
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <div className="ml-3 border-l border-border/50 pl-2">{children}</div>
          </CollapsibleContent>
        </div>
      </Collapsible>
    </FileTreeFolderContext.Provider>
  );
};

interface FileTreeFileContextType {
  path: string;
  name: string;
}

const FileTreeFileContext = createContext<FileTreeFileContextType>({
  name: "",
  path: "",
});

export type FileTreeFileProps = HTMLAttributes<HTMLDivElement> & {
  path: string;
  name: string;
  icon?: ReactNode;
};

export const FileTreeFile = ({
  path,
  name,
  icon,
  className,
  children,
  ...props
}: FileTreeFileProps) => {
  const { selectedPath, onSelect } = useContext(FileTreeContext);
  const isSelected = selectedPath === path;

  const handleClick = useCallback(() => {
    onSelect?.(path);
  }, [onSelect, path]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        onSelect?.(path);
      }
    },
    [onSelect, path],
  );

  return (
    <FileTreeFileContext.Provider value={{ name, path }}>
      <div
        className={cn(
          "group flex cursor-pointer items-center gap-1.5 px-2 py-1 transition-colors",
          "hover:bg-muted/60",
          isSelected && "bg-accent/20 text-accent-foreground",
          className,
        )}
        onClick={handleClick}
        onKeyDown={handleKeyDown}
        role="treeitem"
        tabIndex={0}
        aria-selected={isSelected}
        {...props}
      >
        {children ?? (
          <>
            {/* Spacer for alignment with folder chevrons */}
            <span className="size-3.5" />
            <FileTreeIcon>
              {icon ?? <FileIcon className="size-4 text-muted-foreground" />}
            </FileTreeIcon>
            <FileTreeName>{name}</FileTreeName>
          </>
        )}
      </div>
    </FileTreeFileContext.Provider>
  );
};

export type FileTreeIconProps = HTMLAttributes<HTMLSpanElement>;

export const FileTreeIcon = ({ className, children, ...props }: FileTreeIconProps) => (
  <span className={cn("shrink-0", className)} {...props}>
    {children}
  </span>
);

export type FileTreeNameProps = HTMLAttributes<HTMLSpanElement>;

export const FileTreeName = ({ className, children, ...props }: FileTreeNameProps) => (
  <span className={cn("truncate text-foreground/90", className)} {...props}>
    {children}
  </span>
);

export type FileTreeActionsProps = HTMLAttributes<HTMLDivElement>;

const stopPropagation = (e: React.SyntheticEvent) => e.stopPropagation();

export const FileTreeActions = ({ className, children, ...props }: FileTreeActionsProps) => (
  // biome-ignore lint/a11y/noNoninteractiveElementInteractions: stopPropagation required for nested interactions
  // biome-ignore lint/a11y/useSemanticElements: fieldset doesn't fit this UI pattern
  <div
    className={cn(
      "ml-auto flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity",
      className,
    )}
    onClick={stopPropagation}
    onKeyDown={stopPropagation}
    role="group"
    aria-label="File actions"
    {...props}
  >
    {children}
  </div>
);
