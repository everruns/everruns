/**
 * Decisions:
 * - Notices are a composable primitive so pages can reuse one chrome for info, warning, success, and destructive messaging.
 * - Variants change the accent treatment, while the body text stays readable against the shared card/background tokens.
 */
import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";

import { cn } from "@/lib/utils";

const noticeVariants = cva(
  "border bg-card/90 px-4 py-3 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]",
  {
    variants: {
      variant: {
        info: "border-l-[3px] border-l-primary",
        warning: "border-l-[3px] border-l-accent",
        success: "border-l-[3px] border-l-emerald-600 dark:border-l-emerald-500",
        destructive: "border-l-[3px] border-l-destructive",
      },
      hasIcon: {
        true: "grid grid-cols-[auto_1fr] items-start gap-3",
        false: "block",
      },
    },
    defaultVariants: {
      variant: "info",
      hasIcon: false,
    },
  },
);

const noticeIconVariants = cva("mt-0.5 text-muted-foreground", {
  variants: {
    variant: {
      info: "text-primary",
      warning: "text-accent-foreground",
      success: "text-emerald-700 dark:text-emerald-400",
      destructive: "text-destructive",
    },
  },
  defaultVariants: {
    variant: "info",
  },
});

type NoticeVariant = NonNullable<VariantProps<typeof noticeVariants>["variant"]>;

type NoticeContextValue = {
  variant: NoticeVariant;
};

const NoticeContext = React.createContext<NoticeContextValue | null>(null);

function useNoticeContext() {
  return React.useContext(NoticeContext);
}

type NoticeProps = Omit<React.ComponentProps<"div">, "title"> &
  Omit<VariantProps<typeof noticeVariants>, "hasIcon"> & {
    icon?: React.ReactNode;
  };

const Notice = React.forwardRef<HTMLDivElement, NoticeProps>(
  ({ className, variant, icon, children, ...props }, ref) => {
    const resolvedVariant: NoticeVariant = variant ?? "info";
    const hasIcon = Boolean(icon);

    return (
      <NoticeContext.Provider value={{ variant: resolvedVariant }}>
        <div
          ref={ref}
          data-slot="notice"
          data-variant={resolvedVariant}
          className={cn(noticeVariants({ variant: resolvedVariant, hasIcon }), className)}
          {...props}
        >
          {hasIcon ? (
            <div
              data-slot="notice-icon"
              className={noticeIconVariants({ variant: resolvedVariant })}
            >
              {icon}
            </div>
          ) : null}
          <div className="min-w-0 space-y-1">{children}</div>
        </div>
      </NoticeContext.Provider>
    );
  },
);
Notice.displayName = "Notice";

const noticeTitleVariants = cva("font-medium leading-none text-foreground");

const NoticeTitle = React.forwardRef<HTMLParagraphElement, React.ComponentProps<"p">>(
  ({ className, ...props }, ref) => (
    <p
      ref={ref}
      data-slot="notice-title"
      className={cn(noticeTitleVariants(), className)}
      {...props}
    />
  ),
);
NoticeTitle.displayName = "NoticeTitle";

const noticeDescriptionVariants = cva("text-sm leading-6 text-muted-foreground");

const NoticeDescription = React.forwardRef<HTMLDivElement, React.ComponentProps<"div">>(
  ({ className, ...props }, ref) => {
    const context = useNoticeContext();

    return (
      <div
        ref={ref}
        data-slot="notice-description"
        data-variant={context?.variant}
        className={cn(noticeDescriptionVariants(), className)}
        {...props}
      />
    );
  },
);
NoticeDescription.displayName = "NoticeDescription";

export { Notice, NoticeDescription, NoticeTitle };
