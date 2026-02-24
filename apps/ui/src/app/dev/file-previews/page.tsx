"use client";

import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ArrowLeft } from "lucide-react";
import {
  CodePreview,
  CSVPreview,
  JSONPreview,
  MarkdownPreview,
  ImagePreview,
  canPreview,
  getPreviewType,
} from "@/components/files/file-previews";

// Check if we're in development mode
const isDev = process.env.NODE_ENV === "development";

// --------------------------------------------
// Showcase Section Components
// --------------------------------------------

function ShowcaseSection({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-lg">{title}</CardTitle>
        {description && <CardDescription>{description}</CardDescription>}
      </CardHeader>
      <CardContent className="space-y-4">{children}</CardContent>
    </Card>
  );
}

function ShowcaseItem({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2">
      <div className="text-sm font-medium text-muted-foreground">{label}</div>
      <div className="border rounded-lg overflow-hidden bg-background h-[300px]">{children}</div>
    </div>
  );
}

// --------------------------------------------
// Sample File Contents
// --------------------------------------------

const sampleCode = {
  typescript: `import { useState, useEffect } from 'react';

interface User {
  id: string;
  name: string;
  email: string;
}

export function useUser(userId: string): User | null {
  const [user, setUser] = useState<User | null>(null);

  useEffect(() => {
    // Fetch user data
    async function fetchUser() {
      const response = await fetch(\`/api/users/\${userId}\`);
      const data = await response.json();
      setUser(data);
    }
    fetchUser();
  }, [userId]);

  return user;
}`,

  python: `from dataclasses import dataclass
from typing import Optional
import asyncio

@dataclass
class Task:
    """Represents an async task with metadata."""
    id: str
    name: str
    status: str = "pending"
    result: Optional[str] = None

async def process_task(task: Task) -> Task:
    # Simulate async processing
    await asyncio.sleep(1)
    task.status = "completed"
    task.result = f"Processed {task.name}"
    return task

async def main():
    tasks = [Task(id=str(i), name=f"Task {i}") for i in range(5)]
    results = await asyncio.gather(*[process_task(t) for t in tasks])
    for result in results:
        print(f"{result.name}: {result.status}")

if __name__ == "__main__":
    asyncio.run(main())`,

  rust: `use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct CacheEntry<T> {
    value: T,
    expires_at: u64,
}

pub struct Cache<T> {
    data: RwLock<HashMap<String, CacheEntry<T>>>,
    ttl: u64,
}

impl<T: Clone> Cache<T> {
    pub fn new(ttl: u64) -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    pub async fn get(&self, key: &str) -> Option<T> {
        let data = self.data.read().await;
        data.get(key).map(|entry| entry.value.clone())
    }

    pub async fn set(&self, key: String, value: T) {
        let mut data = self.data.write().await;
        let entry = CacheEntry {
            value,
            expires_at: self.ttl,
        };
        data.insert(key, entry);
    }
}`,

  go: `package main

import (
	"context"
	"fmt"
	"sync"
	"time"
)

// Worker processes tasks from a channel
type Worker struct {
	ID       int
	taskChan chan Task
	wg       *sync.WaitGroup
}

// Task represents a unit of work
type Task struct {
	ID   int
	Data string
}

func (w *Worker) Start(ctx context.Context) {
	defer w.wg.Done()
	for {
		select {
		case task, ok := <-w.taskChan:
			if !ok {
				return
			}
			w.process(task)
		case <-ctx.Done():
			return
		}
	}
}

func (w *Worker) process(task Task) {
	fmt.Printf("Worker %d processing task %d\\n", w.ID, task.ID)
	time.Sleep(100 * time.Millisecond)
}

func main() {
	tasks := make(chan Task, 10)
	var wg sync.WaitGroup

	// Start workers
	for i := 0; i < 3; i++ {
		wg.Add(1)
		worker := &Worker{ID: i, taskChan: tasks, wg: &wg}
		go worker.Start(context.Background())
	}

	// Send tasks
	for i := 0; i < 10; i++ {
		tasks <- Task{ID: i, Data: fmt.Sprintf("data-%d", i)}
	}
	close(tasks)
	wg.Wait()
}`,
};

const sampleCSV = `id,name,email,role,department,salary
1,Alice Johnson,alice@example.com,Engineer,Engineering,95000
2,Bob Smith,bob@example.com,Designer,Design,85000
3,Carol Williams,carol@example.com,Manager,Engineering,120000
4,David Brown,david@example.com,Engineer,Engineering,92000
5,Eve Davis,eve@example.com,Analyst,Data Science,88000
6,Frank Miller,frank@example.com,Director,Product,150000
7,Grace Lee,grace@example.com,Engineer,Engineering,98000
8,Henry Wilson,henry@example.com,Designer,Design,82000`;

const sampleJSON = {
  agent: {
    id: "agent_01H8EXAMPLE",
    name: "Code Assistant",
    status: "active",
    capabilities: ["code_execution", "file_operations", "web_search"],
    config: {
      model: "claude-3-sonnet",
      temperature: 0.7,
      maxTokens: 4096,
    },
    metadata: {
      created_at: "2024-01-15T10:30:00Z",
      updated_at: "2024-01-20T14:45:00Z",
      version: "1.2.0",
    },
  },
  sessions: [
    { id: "session_001", status: "completed", messages: 42 },
    { id: "session_002", status: "active", messages: 15 },
  ],
};

const sampleMarkdown = `# Project Documentation

This is a **sample markdown** file demonstrating various formatting features.

## Features

- *Italic text* and **bold text**
- \`inline code\` for small snippets
- [Links to external resources](https://example.com)

### Code Example

\`\`\`typescript
function hello(name: string): string {
  return \`Hello, \${name}!\`;
}
\`\`\`

## Tables

| Feature | Status | Notes |
|---------|--------|-------|
| Markdown | ✅ Done | Full GFM support |
| Code Highlighting | ✅ Done | Shiki-based |
| Tables | ✅ Done | With styling |

## Alerts

> [!NOTE]
> This is a note alert with important information.

> [!TIP]
> Here's a helpful tip for users.

> [!WARNING]
> Be careful with this operation!

## Conclusion

This demonstrates all the markdown features supported by the preview component.
`;

const sampleMarkdownWithFrontmatter = `---
title: Getting Started Guide
description: A comprehensive guide to setting up and using the platform
author: Jane Smith
date: 2025-12-15
tags: [guide, setup, tutorial]
draft: false
---

# Getting Started

Welcome to the platform! This guide walks you through the initial setup.

## Prerequisites

- Node.js 20 or later
- A GitHub account
- Basic command line knowledge

## Installation

\`\`\`bash
npm install -g everruns
everruns init my-project
\`\`\`

## Next Steps

See the [full documentation](https://docs.example.com) for more details.
`;

// Base64 encoded 1x1 pixel PNG (red)
const sampleImageBase64 =
  "iVBORw0KGgoAAAANSUhEUgAAAGQAAABkCAYAAABw4pVUAAAACXBIWXMAAAsTAAALEwEAmpwYAAABSklEQVR4nO3dwQ2DMBBFUdKROqAPrUOfpoO0YC9YJSJyYmbu3xhZT8ssPgAAAAAAAAAAAABgqftah2HYv8p6us4D0C5lPAAAAACAi5nZ+xWuZd2WMu1wLS2Eg/qjPAD4K89hqF8/R7bHOTQchYOCQMOVIBDwLqshqCEckJ4hdQ/pnKD3FwIAAAAAAAAAgB9qjlJlqThKCKC8tJZeRlDjLAEAAPi/3hdqSKPWhX68UEMaiW2dhkDDbWHbPwIAAAAA4Bfc96OktG7BuicIBNxTSwgH3U8IQAAAAAAAAAAAAAD4qOaofq8/anPUvD4OAO+YK/8V";

// --------------------------------------------
// Main Page Component
// --------------------------------------------

export default function DevFilePreviewsPage() {
  // Show 404-like message in production
  if (!isDev) {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-muted-foreground">404</h1>
          <p className="text-muted-foreground mt-2">Page not found</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-muted/30">
      <div className="container mx-auto py-8 px-4">
        <div className="mb-8">
          <Link
            href="/dev"
            className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-4"
          >
            <ArrowLeft className="w-4 h-4 mr-2" />
            Back to Developer Tools
          </Link>
          <h1 className="text-3xl font-bold">File Preview Components</h1>
          <p className="text-muted-foreground mt-2">
            Preview components for code, CSV, JSON, markdown, and images using Shiki syntax
            highlighting
          </p>
          <Badge variant="outline" className="mt-2">
            Development Mode
          </Badge>
        </div>

        <div className="space-y-8">
          {/* Code Preview - Multiple Languages */}
          <ShowcaseSection
            title="Code Preview"
            description="Shiki-based syntax highlighting for various programming languages"
          >
            <ShowcaseItem label="TypeScript (.ts)">
              <CodePreview content={sampleCode.typescript} extension="ts" />
            </ShowcaseItem>

            <ShowcaseItem label="Python (.py)">
              <CodePreview content={sampleCode.python} extension="py" />
            </ShowcaseItem>

            <ShowcaseItem label="Rust (.rs)">
              <CodePreview content={sampleCode.rust} extension="rs" />
            </ShowcaseItem>

            <ShowcaseItem label="Go (.go)">
              <CodePreview content={sampleCode.go} extension="go" />
            </ShowcaseItem>
          </ShowcaseSection>

          {/* CSV Preview */}
          <ShowcaseSection
            title="CSV Preview"
            description="Tabular data rendered as formatted tables"
          >
            <ShowcaseItem label="Employee Data (.csv)">
              <CSVPreview content={sampleCSV} />
            </ShowcaseItem>
          </ShowcaseSection>

          {/* JSON Preview */}
          <ShowcaseSection
            title="JSON Preview"
            description="Pretty-printed JSON with Shiki syntax highlighting"
          >
            <ShowcaseItem label="Agent Configuration (.json)">
              <JSONPreview content={JSON.stringify(sampleJSON)} />
            </ShowcaseItem>
          </ShowcaseSection>

          {/* Markdown Preview */}
          <ShowcaseSection
            title="Markdown Preview"
            description="Full markdown rendering with GFM support and code highlighting"
          >
            <ShowcaseItem label="Documentation (.md)">
              <MarkdownPreview content={sampleMarkdown} />
            </ShowcaseItem>

            <ShowcaseItem label="With Frontmatter (.md)">
              <MarkdownPreview content={sampleMarkdownWithFrontmatter} />
            </ShowcaseItem>
          </ShowcaseSection>

          {/* Image Preview */}
          <ShowcaseSection
            title="Image Preview"
            description="Inline image rendering from base64-encoded content"
          >
            <ShowcaseItem label="Sample Image (.png)">
              <ImagePreview content={sampleImageBase64} extension="png" fileName="sample.png" />
            </ShowcaseItem>
          </ShowcaseSection>

          {/* FilePreview Unified Component */}
          <ShowcaseSection
            title="Unified FilePreview Component"
            description="Automatic preview type detection based on file extension"
          >
            <div className="grid grid-cols-2 gap-4 text-sm">
              <div>
                <div className="font-medium mb-2">Preview Type Detection</div>
                <table className="w-full border-collapse">
                  <thead>
                    <tr className="border-b">
                      <th className="text-left py-1">Extension</th>
                      <th className="text-left py-1">Type</th>
                      <th className="text-left py-1">Can Preview</th>
                    </tr>
                  </thead>
                  <tbody>
                    {["ts", "py", "rs", "csv", "json", "md", "png", "txt", "exe"].map((ext) => (
                      <tr key={ext} className="border-b">
                        <td className="py-1">.{ext}</td>
                        <td className="py-1">
                          {getPreviewType(ext, ext === "png" ? "base64" : "text")}
                        </td>
                        <td className="py-1">
                          {canPreview(ext, ext === "png" ? "base64" : "text") ? "✅" : "❌"}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <div>
                <div className="font-medium mb-2">Usage</div>
                <pre className="bg-muted p-4 rounded-md text-xs overflow-x-auto">
                  {`import { FilePreview, canPreview } from "@/components/files";

// Check if preview is available
if (canPreview(extension, encoding)) {
  return (
    <FilePreview
      content={fileContent}
      extension={extension}
      encoding={encoding}
    />
  );
}`}
                </pre>
              </div>
            </div>
          </ShowcaseSection>

          {/* Supported Languages */}
          <ShowcaseSection
            title="Supported Languages"
            description="Code languages with Shiki syntax highlighting"
          >
            <div className="flex flex-wrap gap-2">
              {[
                "js",
                "jsx",
                "ts",
                "tsx",
                "py",
                "rs",
                "go",
                "java",
                "c",
                "cpp",
                "rb",
                "php",
                "sql",
                "sh",
                "bash",
                "yml",
                "yaml",
                "toml",
                "xml",
                "html",
                "css",
                "scss",
                "vue",
                "svelte",
                "json",
              ].map((lang) => (
                <Badge key={lang} variant="secondary">
                  .{lang}
                </Badge>
              ))}
            </div>
          </ShowcaseSection>
        </div>
      </div>
    </div>
  );
}
