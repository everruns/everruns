"use client";

import { SessionIcon } from "@/components/icons/facet-icons";
import { Info, Loader2, Zap, Pin, PinOff, CalendarClock, Download } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { EntityCard } from "@/components/ui/entity-card";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { Tooltip, TooltipTrigger, TooltipContent } from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuPositioner,
  DropdownMenuContent,
  DropdownMenuItem,
} from "@/components/ui/dropdown-menu";
import { cn, shortenId } from "@/lib/utils";
import { formatRelativeTime, formatTokens } from "@/lib/formatting";
import { useLocale } from "@/providers/locale-provider";
import type { Session, ModelWithProvider, TokenUsage } from "@/lib/api/types";
import type { SessionExportFormat } from "@/lib/api/sessions";
import { getEntityReferenceClassName, getEntityReferenceLabel } from "@/lib/entity-lifecycle";
import { joinTags } from "@/lib/tags";
import { sessionOutcome } from "@/lib/session-outcome";
import Link from "next/link";
import { failureTraceHref } from "@/lib/agent-failures";
import { sourceLabel } from "@/lib/session-filters";
