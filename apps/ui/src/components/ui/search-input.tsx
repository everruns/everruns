import { Search } from "lucide-react";
import * as React from "react";

import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

type SearchInputProps = React.ComponentProps<typeof Input> & {
  /** Classes for the wrapping element. Put width/flex utilities here (e.g. `w-64`). */
  containerClassName?: string;
};

/**
 * Search field used across list/index pages. Wraps {@link Input} with a
 * vertically-centered magnifier icon so every search box renders at the same
 * height as sibling buttons (the underlying Input is `h-8`, matching the
 * default Button). Centralizing this keeps icon alignment and height
 * consistent instead of each page hand-rolling the `relative`/`absolute` markup.
 */
function SearchInput({ className, containerClassName, children, ...props }: SearchInputProps) {
  return (
    <div className={cn("relative", containerClassName)}>
      <Search className="text-muted-foreground pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2" />
      <Input data-slot="search-input" className={cn("pl-8", className)} {...props} />
      {children}
    </div>
  );
}

export { SearchInput };
