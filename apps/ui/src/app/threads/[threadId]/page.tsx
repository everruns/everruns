"use client";

import { useParams } from "next/navigation";
import Link from "next/link";
import { useThread, useMessages } from "@/hooks/use-threads";
import { Header } from "@/components/layout/header";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Skeleton } from "@/components/ui/skeleton";
import { ArrowLeft, Bot, User, MessageSquare } from "lucide-react";
import { cn } from "@/lib/utils";
import type { Message } from "@/lib/api/types";

function MessageItem({ message }: { message: Message }) {
  const isUser = message.role === "user";

  return (
    <div className={cn("flex gap-4 p-4", isUser ? "bg-muted/50" : "bg-background")}>
      <Avatar className="h-8 w-8 shrink-0">
        <AvatarFallback className={cn(isUser ? "bg-primary" : "bg-muted")}>
          {isUser ? <User className="h-4 w-4" /> : <Bot className="h-4 w-4" />}
        </AvatarFallback>
      </Avatar>
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 mb-1">
          <span className="font-medium text-sm capitalize">{message.role}</span>
          <span className="text-xs text-muted-foreground">
            {new Date(message.created_at).toLocaleString()}
          </span>
        </div>
        <p className="whitespace-pre-wrap text-sm">{message.content}</p>
        {message.metadata && Object.keys(message.metadata).length > 0 && (
          <pre className="mt-2 text-xs bg-muted p-2 rounded overflow-x-auto">
            {JSON.stringify(message.metadata, null, 2)}
          </pre>
        )}
      </div>
    </div>
  );
}

export default function ThreadDetailPage() {
  const params = useParams();
  const threadId = params.threadId as string;

  const { data: thread, isLoading: threadLoading, error: threadError } = useThread(threadId);
  const { data: messages = [], isLoading: messagesLoading } = useMessages(threadId);

  const isLoading = threadLoading || messagesLoading;

  if (isLoading) {
    return (
      <>
        <Header title="Thread" />
        <div className="p-6 space-y-6">
          <Skeleton className="h-24" />
          <Skeleton className="h-64" />
        </div>
      </>
    );
  }

  if (threadError || !thread) {
    return (
      <>
        <Header
          title="Thread Not Found"
          action={
            <Link href="/runs">
              <Button variant="ghost">
                <ArrowLeft className="h-4 w-4 mr-2" />
                Back to Runs
              </Button>
            </Link>
          }
        />
        <div className="p-6">
          <div className="bg-destructive/10 text-destructive p-4 rounded-lg">
            {threadError?.message || "Thread not found"}
          </div>
        </div>
      </>
    );
  }

  return (
    <>
      <Header
        title={`Thread ${thread.id.slice(0, 8)}...`}
        action={
          <div className="flex gap-2">
            <Link href="/runs">
              <Button variant="ghost">
                <ArrowLeft className="h-4 w-4 mr-2" />
                Back
              </Button>
            </Link>
            <Link href={`/chat?thread=${thread.id}`}>
              <Button variant="outline">
                <MessageSquare className="h-4 w-4 mr-2" />
                Continue in Chat
              </Button>
            </Link>
          </div>
        }
      />
      <div className="p-6 space-y-6">
        {/* Thread Info Card */}
        <Card>
          <CardHeader>
            <CardTitle className="font-mono text-lg">{thread.id}</CardTitle>
            <CardDescription>
              Created {new Date(thread.created_at).toLocaleString()}
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="text-sm text-muted-foreground">
              {messages.length} message{messages.length !== 1 && "s"}
            </div>
          </CardContent>
        </Card>

        {/* Messages */}
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Messages</CardTitle>
          </CardHeader>
          <CardContent className="p-0">
            {messages.length === 0 ? (
              <div className="text-center py-12 text-muted-foreground">
                <MessageSquare className="h-12 w-12 mx-auto mb-4" />
                <p>No messages in this thread</p>
              </div>
            ) : (
              <div className="divide-y">
                {messages.map((message) => (
                  <MessageItem key={message.id} message={message} />
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </>
  );
}
