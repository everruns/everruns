"use client";

import * as React from "react";
import { Select as SelectPrimitive } from "@base-ui/react/select";
import { CheckIcon, ChevronDownIcon } from "lucide-react";

import { cn } from "@/lib/utils";

// Select must always show the human-readable label of the selected item, never
// the raw internal value (e.g. show "Webhook", not "webhook"). base-ui resolves
// labels from <Select.Value> using its `items` prop, so the wrapper auto-walks
// <SelectItem> children and forwards a value→label map by default.
// Spec: knowledge/foundations/code-organization.md "Dropdowns must show display names".
type SelectItemsMap = Record<string, React.ReactNode>;

function collectSelectItemLabels(
  node: React.ReactNode,
  acc: SelectItemsMap,
  visited: WeakSet<object>,
): void {
  React.Children.forEach(node, (child) => {
    if (!React.isValidElement(child)) return;
    if (visited.has(child)) return;
    visited.add(child);

    if (child.type === SelectItem) {
      const props = child.props as { value?: unknown; children?: React.ReactNode };
      if (props.value !== undefined && props.value !== null) {
        const key = String(props.value);
        if (!(key in acc)) {
          acc[key] = props.children;
        }
      }
      // SelectItem children are the label content — don't descend further.
      return;
    }

    const grandChildren = (child.props as { children?: React.ReactNode } | null)?.children;
    if (grandChildren !== undefined && grandChildren !== null) {
      collectSelectItemLabels(grandChildren, acc, visited);
    }
  });
}

interface SelectProps extends Omit<SelectPrimitive.Root.Props<string>, "onValueChange" | "items"> {
  onValueChange?: (value: string) => void;
  /**
   * Optional explicit value→label map. When omitted, labels are auto-collected
   * from <SelectItem> children so <SelectValue> always shows the display name.
   */
  items?: SelectItemsMap | ReadonlyArray<{ value: string; label: React.ReactNode }>;
}

function Select({ onValueChange, items, children, ...props }: SelectProps) {
  const handleValueChange = React.useCallback(
    (value: string | null) => {
      if (value !== null && onValueChange) {
        onValueChange(value);
      }
    },
    [onValueChange],
  );

  // Computed inline on every render: children is a fresh ReactNode tree each
  // render, so memoizing on [items, children] would invalidate every time.
  // The traversal is bounded by the number of <SelectItem> children — tiny
  // for typical selects.
  const itemsMap: SelectItemsMap = {};
  if (items) {
    if (Array.isArray(items)) {
      for (const item of items) itemsMap[item.value] = item.label;
    } else {
      Object.assign(itemsMap, items as SelectItemsMap);
    }
  }
  collectSelectItemLabels(children, itemsMap, new WeakSet());

  return (
    <SelectPrimitive.Root<string> onValueChange={handleValueChange} items={itemsMap} {...props}>
      {children}
    </SelectPrimitive.Root>
  );
}

function SelectGroup({ ...props }: SelectPrimitive.Group.Props) {
  return <SelectPrimitive.Group data-slot="select-group" {...props} />;
}

// Renders the label of the selected item. base-ui resolves the label from the
// `items` map provided by <Select> above. Pass children only when overriding
// the default lookup (e.g. compose icon + label in the trigger).
function SelectValue({
  placeholder,
  children,
  ...props
}: SelectPrimitive.Value.Props & {
  placeholder?: React.ReactNode;
  children?: React.ReactNode | ((value: unknown) => React.ReactNode);
}) {
  return (
    <SelectPrimitive.Value data-slot="select-value" placeholder={placeholder} {...props}>
      {children}
    </SelectPrimitive.Value>
  );
}

function SelectTrigger({
  className,
  size = "default",
  children,
  ...props
}: SelectPrimitive.Trigger.Props & {
  size?: "sm" | "default";
}) {
  return (
    <SelectPrimitive.Trigger
      data-slot="select-trigger"
      data-size={size}
      className={cn(
        "border-input data-[placeholder]:text-muted-foreground [&_svg:not([class*='text-'])]:text-muted-foreground focus-visible:border-ring focus-visible:ring-ring/50 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive dark:bg-input/30 dark:hover:bg-input/50 flex w-fit appearance-none items-center justify-between gap-2 border bg-transparent px-2.5 py-1.5 text-[13px] whitespace-nowrap shadow-xs transition-[color,box-shadow] outline-none focus-visible:ring-[3px] disabled:cursor-not-allowed disabled:opacity-50 data-[size=default]:h-8 data-[size=sm]:h-7 *:data-[slot=select-value]:line-clamp-1 *:data-[slot=select-value]:flex *:data-[slot=select-value]:items-center *:data-[slot=select-value]:gap-2 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
        className,
      )}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon render={<ChevronDownIcon className="size-4 opacity-50" />} />
    </SelectPrimitive.Trigger>
  );
}

function SelectPositioner({ className, ...props }: SelectPrimitive.Positioner.Props) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Positioner
        data-slot="select-positioner"
        alignItemWithTrigger={false}
        sideOffset={5}
        className={cn("z-50", className)}
        {...props}
      />
    </SelectPrimitive.Portal>
  );
}

function SelectContent({ className, children, ...props }: SelectPrimitive.Popup.Props) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Positioner data-slot="select-positioner" sideOffset={5} className="z-50">
        <SelectPrimitive.Popup
          data-slot="select-content"
          className={cn(
            "p-1 bg-popover text-popover-foreground data-[open]:animate-in data-[closed]:animate-out data-[closed]:fade-out-0 data-[open]:fade-in-0 data-[closed]:zoom-out-95 data-[open]:zoom-in-95 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 relative z-50 max-h-(--available-height) min-w-(--anchor-width) origin-(--transform-origin) overflow-x-hidden overflow-y-auto border shadow-md",
            className,
          )}
          {...props}
        >
          {children}
        </SelectPrimitive.Popup>
      </SelectPrimitive.Positioner>
    </SelectPrimitive.Portal>
  );
}

function SelectLabel({ className, ...props }: SelectPrimitive.GroupLabel.Props) {
  return (
    <SelectPrimitive.GroupLabel
      data-slot="select-label"
      className={cn(
        "text-muted-foreground px-2 py-1 text-[11px] uppercase tracking-[0.14em]",
        className,
      )}
      {...props}
    />
  );
}

function SelectItem({ className, children, ...props }: SelectPrimitive.Item.Props) {
  return (
    <SelectPrimitive.Item
      data-slot="select-item"
      className={cn(
        "focus:bg-muted focus:text-foreground [&_svg:not([class*='text-'])]:text-muted-foreground relative flex w-full cursor-default items-center gap-2 py-1 pr-8 pl-2 text-[13px] outline-hidden select-none data-[disabled]:pointer-events-none data-[disabled]:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 *:[span]:last:flex *:[span]:last:items-center *:[span]:last:gap-2",
        className,
      )}
      {...props}
    >
      <span className="absolute right-2 flex size-3.5 items-center justify-center">
        <SelectPrimitive.ItemIndicator>
          <CheckIcon className="size-4" />
        </SelectPrimitive.ItemIndicator>
      </span>
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
    </SelectPrimitive.Item>
  );
}

function SelectSeparator({ className, ...props }: SelectPrimitive.Separator.Props) {
  return (
    <SelectPrimitive.Separator
      data-slot="select-separator"
      className={cn("bg-border pointer-events-none -mx-1 my-1 h-px", className)}
      {...props}
    />
  );
}

export {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
  SelectPositioner,
};
