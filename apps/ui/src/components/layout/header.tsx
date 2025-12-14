"use client";

interface HeaderProps {
  title: string;
  description?: string;
  action?: React.ReactNode;
  actions?: React.ReactNode;
}

export function Header({ title, description, action, actions }: HeaderProps) {
  const headerActions = action || actions;

  return (
    <div className="flex items-center justify-between border-b bg-card px-6 py-4">
      <div>
        <h1 className="text-2xl font-bold">{title}</h1>
        {description && (
          <p className="text-sm text-muted-foreground">{description}</p>
        )}
      </div>
      {headerActions && <div className="flex items-center gap-2">{headerActions}</div>}
    </div>
  );
}
