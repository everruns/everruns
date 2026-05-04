import { AlertCircle } from "lucide-react";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";

type DeleteAction = "archive" | "delete";

interface EntityDeleteErrorNoticeProps {
  entityKind: "agent" | "harness" | "identity";
  action: DeleteAction;
  message: string;
  className?: string;
}

function titleFor(entityKind: EntityDeleteErrorNoticeProps["entityKind"], action: DeleteAction) {
  const label = entityKind === "identity" ? "Agent Identity" : capitalize(entityKind);
  return `Cannot ${capitalize(action)} ${label}`;
}

function capitalize(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1);
}

function formatDeleteErrorMessage(
  entityKind: EntityDeleteErrorNoticeProps["entityKind"],
  action: DeleteAction,
  message: string,
) {
  const appReference = message.match(
    /^Cannot archive or delete (agent|harness|agent identity) while apps still reference it: (.+)$/i,
  );
  if (appReference) {
    const apps = appReference[2];
    const label = entityKind === "identity" ? "agent identity" : entityKind;
    const verb = action === "archive" ? "archiving" : "deleting";
    return `This ${label} is still used by apps: ${apps}. Remove it from those apps before ${verb}.`;
  }

  const childHarnesses = message.match(
    /^Cannot archive or delete harness while child harnesses still inherit from it: (.+)$/i,
  );
  if (childHarnesses) {
    const verb = action === "archive" ? "archiving" : "deleting";
    return `Other harnesses still inherit from this harness: ${childHarnesses[1]}. Move or remove those child harnesses before ${verb}.`;
  }

  if (message === "Cannot archive or delete harness while it is the organization default harness") {
    const verb = action === "archive" ? "archiving" : "deleting";
    return `This harness is the organization default harness. Choose a different default harness before ${verb}.`;
  }

  if (message === "Cannot archive or delete harness while it is the organization base harness") {
    const verb = action === "archive" ? "archiving" : "deleting";
    return `This harness is the organization base harness. Choose a different base harness before ${verb}.`;
  }

  return message;
}

export function EntityDeleteErrorNotice({
  entityKind,
  action,
  message,
  className,
}: EntityDeleteErrorNoticeProps) {
  return (
    <Notice variant="destructive" className={className} icon={<AlertCircle className="size-4" />}>
      <NoticeTitle>{titleFor(entityKind, action)}</NoticeTitle>
      <NoticeDescription>{formatDeleteErrorMessage(entityKind, action, message)}</NoticeDescription>
    </Notice>
  );
}
