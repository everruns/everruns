"use client";

import { useState } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Moon, Sun, Check } from "lucide-react";

const isDev = process.env.NODE_ENV === "development";

const backgrounds = [
  {
    id: "none",
    name: "None (Default)",
    description: "No background pattern - clean solid color",
    className: "",
  },
  {
    id: "dots",
    name: "Subtle Dot Grid",
    description: "Tiny dots in a grid pattern using brand navy/gold",
    className: "bg-brand-dots",
  },
  {
    id: "lines",
    name: "Diagonal Lines",
    description: "Faint diagonal lines - technical/engineering feel",
    className: "bg-brand-lines",
  },
  {
    id: "gradient",
    name: "Corner Gradient",
    description: "Subtle radial gradients from corners - adds depth",
    className: "bg-brand-gradient",
  },
  {
    id: "rings",
    name: "Interlocking Rings",
    description: "Inspired by the Borromean rings logo - subtle circles",
    className: "bg-brand-rings",
  },
  {
    id: "grid",
    name: "Fine Grid",
    description: "Subtle crosshatch grid - clean, technical aesthetic",
    className: "bg-brand-grid",
  },
  {
    id: "noise",
    name: "Noise Texture",
    description: "Very fine noise pattern - texture without pattern",
    className: "bg-brand-noise",
  },
];

export default function BackgroundsPage() {
  const [darkMode, setDarkMode] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);

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
    <div className={darkMode ? "dark" : ""}>
      <div className="min-h-screen bg-background text-foreground">
        <div className="container mx-auto py-8 px-4">
          <div className="mb-8 flex items-center justify-between">
            <div>
              <h1 className="text-3xl font-bold mb-2">Branded Background Options</h1>
              <p className="text-muted-foreground">
                Choose a barely visible background pattern for the app.
                All options work for both light and dark modes.
              </p>
            </div>
            <Button
              variant="outline"
              size="icon"
              onClick={() => setDarkMode(!darkMode)}
              aria-label="Toggle dark mode"
            >
              {darkMode ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
            </Button>
          </div>

          {selected && (
            <div className="mb-6 p-4 border rounded-lg bg-muted/50">
              <p className="text-sm text-muted-foreground">
                Selected: <strong>{backgrounds.find(b => b.id === selected)?.name}</strong>
              </p>
              <p className="text-xs text-muted-foreground mt-1 font-mono">
                Class: <code>{backgrounds.find(b => b.id === selected)?.className || "(none)"}</code>
              </p>
            </div>
          )}

          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {backgrounds.map((bg) => (
              <Card
                key={bg.id}
                className={`cursor-pointer transition-all hover:ring-2 hover:ring-primary/50 ${
                  selected === bg.id ? "ring-2 ring-primary" : ""
                }`}
                onClick={() => setSelected(bg.id)}
              >
                <CardHeader className="pb-2">
                  <div className="flex items-center justify-between">
                    <CardTitle className="text-base">{bg.name}</CardTitle>
                    {selected === bg.id && (
                      <Check className="h-4 w-4 text-primary" />
                    )}
                  </div>
                  <p className="text-xs text-muted-foreground">{bg.description}</p>
                </CardHeader>
                <CardContent>
                  {/* Preview box */}
                  <div
                    className={`h-32 rounded-md border ${bg.className} ${
                      darkMode ? "bg-[hsl(0_0%_3.9%)]" : "bg-white"
                    }`}
                  >
                    {/* Sample content to show how bg looks with content */}
                    <div className="p-3 space-y-2">
                      <div
                        className={`h-2 w-20 rounded ${
                          darkMode ? "bg-white/10" : "bg-black/5"
                        }`}
                      />
                      <div
                        className={`h-2 w-32 rounded ${
                          darkMode ? "bg-white/10" : "bg-black/5"
                        }`}
                      />
                      <div
                        className={`h-2 w-16 rounded ${
                          darkMode ? "bg-white/10" : "bg-black/5"
                        }`}
                      />
                    </div>
                  </div>
                  {bg.className && (
                    <Badge variant="secondary" className="mt-2 text-xs font-mono">
                      .{bg.className}
                    </Badge>
                  )}
                </CardContent>
              </Card>
            ))}
          </div>

          {/* Full-page preview */}
          <div className="mt-8">
            <h2 className="text-xl font-semibold mb-4">Full-Width Preview</h2>
            <p className="text-sm text-muted-foreground mb-4">
              Click a card above to see how it looks at full width
            </p>
            <div
              className={`min-h-[400px] rounded-lg border ${
                selected ? backgrounds.find(b => b.id === selected)?.className : ""
              } ${darkMode ? "bg-[hsl(0_0%_3.9%)]" : "bg-white"}`}
            >
              <div className="p-8">
                <h3 className={`text-2xl font-bold mb-4 ${darkMode ? "text-white" : "text-black"}`}>
                  Sample Content
                </h3>
                <p className={`mb-4 ${darkMode ? "text-white/70" : "text-black/70"}`}>
                  This is how your content will look with the selected background.
                  The pattern should be barely visible - just enough to add subtle
                  texture without distracting from the content.
                </p>
                <div className="flex gap-2">
                  <Button>Primary Action</Button>
                  <Button variant="outline">Secondary</Button>
                </div>
              </div>
            </div>
          </div>

          {/* Usage instructions */}
          <div className="mt-8 p-4 border rounded-lg bg-muted/50">
            <h3 className="font-semibold mb-2">Usage</h3>
            <p className="text-sm text-muted-foreground mb-2">
              Once you choose a background, add the class to the body element in your layout:
            </p>
            <pre className="text-xs bg-black/5 dark:bg-white/5 p-2 rounded font-mono">
{`// In layout.tsx
<body className={\`\${geistSans.variable} ... bg-brand-dots\`}>`}
            </pre>
          </div>
        </div>
      </div>
    </div>
  );
}
