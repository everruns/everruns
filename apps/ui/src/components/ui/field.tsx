import type { ReactNode } from "react";
import { Label } from "./label";
import { cn } from "@/lib/utils";

// Pass `${id}-hint` to the control's aria-describedby; errors take precedence.
export function Field({
  id,
  label,
  description,
  error,
  children,
  className,
}: {
  id: string;
  label: ReactNode;
  description?: ReactNode;
  error?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("space-y-1.5", className)}>
      <Label htmlFor={id}>{label}</Label>
      {children}
      {(error || description) && (
        <p
          id={`${id}-hint`}
          role={error ? "alert" : undefined}
          className={cn("text-xs", error ? "text-destructive" : "text-muted-foreground")}
        >
          {error || description}
        </p>
      )}
    </div>
  );
}
