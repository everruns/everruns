"use client";

import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { FlaskConical, MessageSquare, ArrowRight } from "lucide-react";

// Check if we're in development mode
const isDev = process.env.NODE_ENV === "development";

const devPages = [
  {
    title: "Session Chat Components",
    description: "Components used in the Session UI for chat messages, tool calls, and todo lists",
    href: "/dev/components",
    icon: MessageSquare,
  },
];

export default function DevPage() {
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
          <div className="flex items-center gap-2 mb-2">
            <FlaskConical className="h-8 w-8" />
            <h1 className="text-3xl font-bold">Developer Tools</h1>
          </div>
          <p className="text-muted-foreground">
            Development-only pages for testing and previewing UI components
          </p>
          <Badge variant="outline" className="mt-2">
            Development Mode
          </Badge>
        </div>

        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {devPages.map((page) => (
            <Link key={page.href} href={page.href}>
              <Card className="h-full hover:border-primary transition-colors cursor-pointer">
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <page.icon className="h-5 w-5 text-muted-foreground" />
                    <ArrowRight className="h-4 w-4 text-muted-foreground" />
                  </div>
                  <CardTitle className="text-lg">{page.title}</CardTitle>
                  <CardDescription>{page.description}</CardDescription>
                </CardHeader>
                <CardContent />
              </Card>
            </Link>
          ))}
        </div>
      </div>
    </div>
  );
}
