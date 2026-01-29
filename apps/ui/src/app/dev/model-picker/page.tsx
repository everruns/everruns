"use client";

import { useState } from "react";
import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ArrowLeft, Star } from "lucide-react";
import { ModelPicker, FavoriteToggle } from "@/components/models/model-picker";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { useLlmModels } from "@/hooks/use-llm-providers";
import { Skeleton } from "@/components/ui/skeleton";

// Check if we're in development mode
const isDev = process.env.NODE_ENV === "development";

// ============================================
// Showcase Section Components
// ============================================

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
      <div className="border rounded-lg p-4 bg-background">{children}</div>
    </div>
  );
}

// ============================================
// Model List Component
// ============================================

function ModelList() {
  const { data: models = [], isLoading } = useLlmModels();

  if (isLoading) {
    return (
      <div className="space-y-2">
        {[1, 2, 3, 4, 5].map((i) => (
          <Skeleton key={i} className="h-12 w-full" />
        ))}
      </div>
    );
  }

  // Group by favorite status
  const favoriteModels = models.filter((m) => m.is_favorite);
  const otherModels = models.filter((m) => !m.is_favorite);

  return (
    <div className="space-y-4">
      {favoriteModels.length > 0 && (
        <div className="space-y-2">
          <h4 className="text-sm font-medium text-muted-foreground flex items-center gap-2">
            <Star className="w-4 h-4 fill-yellow-400 text-yellow-400" />
            Favorites
          </h4>
          <div className="space-y-1">
            {favoriteModels.map((model) => (
              <div
                key={model.id}
                className="flex items-center gap-3 p-2 rounded-md border bg-muted/30"
              >
                <ProviderIcon providerType={model.provider_type} size="sm" showBackground={false} />
                <div className="flex-1">
                  <div className="font-medium text-sm">{model.display_name}</div>
                  <div className="text-xs text-muted-foreground">
                    {model.provider_name} &middot; {model.model_id}
                  </div>
                </div>
                <FavoriteToggle model={model} size="sm" />
                {model.is_default && (
                  <Badge variant="secondary" className="text-xs">
                    Default
                  </Badge>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {otherModels.length > 0 && (
        <div className="space-y-2">
          <h4 className="text-sm font-medium text-muted-foreground">Other Models</h4>
          <div className="space-y-1">
            {otherModels.map((model) => (
              <div key={model.id} className="flex items-center gap-3 p-2 rounded-md border">
                <ProviderIcon providerType={model.provider_type} size="sm" showBackground={false} />
                <div className="flex-1">
                  <div className="font-medium text-sm">{model.display_name}</div>
                  <div className="text-xs text-muted-foreground">
                    {model.provider_name} &middot; {model.model_id}
                  </div>
                </div>
                <FavoriteToggle model={model} size="sm" />
                {model.is_default && (
                  <Badge variant="secondary" className="text-xs">
                    Default
                  </Badge>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// ============================================
// Main Page Component
// ============================================

export default function DevModelPickerPage() {
  const [selectedModel1, setSelectedModel1] = useState<string>("");
  const [selectedModel2, setSelectedModel2] = useState<string>("");

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
          <h1 className="text-3xl font-bold">Model Picker</h1>
          <p className="text-muted-foreground mt-2">
            LLM Model selection component with favorite models support
          </p>
          <Badge variant="outline" className="mt-2">
            Development Mode
          </Badge>
        </div>

        <ScrollArea className="h-[calc(100vh-12rem)]">
          <div className="space-y-8 pr-4">
            {/* Model Picker Component */}
            <ShowcaseSection
              title="ModelPicker Component"
              description="Dropdown selector for LLM models with favorites shown first"
            >
              <ShowcaseItem label="Basic Model Picker">
                <div className="max-w-md">
                  <ModelPicker
                    value={selectedModel1}
                    onChange={setSelectedModel1}
                    placeholder="Select a model"
                  />
                  <p className="text-xs text-muted-foreground mt-2">
                    Selected: {selectedModel1 || "(none)"}
                  </p>
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="With Custom Placeholder">
                <div className="max-w-md">
                  <ModelPicker
                    value={selectedModel2}
                    onChange={setSelectedModel2}
                    placeholder="Use default model"
                  />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Disabled State">
                <div className="max-w-md">
                  <ModelPicker value="" onChange={() => {}} placeholder="Select a model" disabled />
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Model List with Favorite Toggle */}
            <ShowcaseSection
              title="Model List with FavoriteToggle"
              description="List view of all models with toggle buttons to add/remove favorites"
            >
              <ShowcaseItem label="All Available Models">
                <ModelList />
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Usage Example */}
            <ShowcaseSection
              title="Usage Example"
              description="How to use the ModelPicker in your components"
            >
              <ShowcaseItem label="Code Example">
                <pre className="text-sm bg-muted p-4 rounded-md overflow-x-auto">
                  {`import { ModelPicker } from "@/components/models/model-picker";

function MyComponent() {
  const [modelId, setModelId] = useState("");

  return (
    <ModelPicker
      value={modelId}
      onChange={setModelId}
      placeholder="Select a model"
    />
  );
}`}
                </pre>
              </ShowcaseItem>

              <ShowcaseItem label="FavoriteToggle Example">
                <pre className="text-sm bg-muted p-4 rounded-md overflow-x-auto">
                  {`import { FavoriteToggle } from "@/components/models/model-picker";

function ModelRow({ model }) {
  return (
    <div className="flex items-center gap-2">
      <span>{model.display_name}</span>
      <FavoriteToggle model={model} size="sm" />
    </div>
  );
}`}
                </pre>
              </ShowcaseItem>
            </ShowcaseSection>
          </div>
        </ScrollArea>
      </div>
    </div>
  );
}
