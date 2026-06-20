import { Server } from "lucide-react";
import type { DriverId } from "@/lib/api/types";
import { cn } from "@/lib/utils";

function OpenAiIcon({ size }: { size: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
    >
      <path
        d="M22.2819 9.8211a5.9847 5.9847 0 0 0-.5157-4.9108 6.0462 6.0462 0 0 0-6.5098-2.9A6.0651 6.0651 0 0 0 4.9807 4.1818a5.9847 5.9847 0 0 0-3.9977 2.9 6.0462 6.0462 0 0 0 .7427 7.0966 5.98 5.98 0 0 0 .511 4.9107 6.051 6.051 0 0 0 6.5146 2.9001A5.9847 5.9847 0 0 0 13.2599 24a6.0557 6.0557 0 0 0 5.7718-4.2058 5.9894 5.9894 0 0 0 3.9977-2.9001 6.0557 6.0557 0 0 0-.7475-7.0729zm-9.022 12.6081a4.4755 4.4755 0 0 1-2.8764-1.0408l.1419-.0804 4.7783-2.7582a.7948.7948 0 0 0 .3927-.6813v-6.7369l2.02 1.1686a.071.071 0 0 1 .038.052v5.5826a4.504 4.504 0 0 1-4.4945 4.4944zm-9.6607-4.1254a4.4708 4.4708 0 0 1-.5346-3.0137l.142.0852 4.783 2.7582a.7712.7712 0 0 0 .7806 0l5.8428-3.3685v2.3324a.0804.0804 0 0 1-.0332.0615L9.74 19.9502a4.4992 4.4992 0 0 1-6.1408-1.6464zM2.3408 7.8956a4.485 4.485 0 0 1 2.3655-1.9728V11.6a.7664.7664 0 0 0 .3879.6765l5.8144 3.3543-2.0201 1.1685a.0757.0757 0 0 1-.071 0l-4.8303-2.7865A4.504 4.504 0 0 1 2.3408 7.872zm16.5963 3.8558L13.1038 8.364l2.0201-1.1638a.0757.0757 0 0 1 .071 0l4.8303 2.7913a4.4944 4.4944 0 0 1-.6765 8.1042v-5.6772a.79.79 0 0 0-.4043-.6813zm2.0107-3.0231l-.142-.0852-4.7735-2.7818a.7759.7759 0 0 0-.7854 0L9.409 9.2297V6.8974a.0662.0662 0 0 1 .0284-.0615l4.8303-2.7866a4.4992 4.4992 0 0 1 6.6802 4.66zM8.3065 12.863l-2.02-1.1638a.0804.0804 0 0 1-.038-.0567V6.0742a4.4992 4.4992 0 0 1 7.3757-3.4537l-.142.0805L8.704 5.459a.7948.7948 0 0 0-.3927.6813zm1.0976-2.3654l2.602-1.4998 2.6069 1.4998v2.9994l-2.5974 1.4997-2.6067-1.4997Z"
        fill="currentColor"
      />
    </svg>
  );
}

function OpenRouterIcon({ size }: { size: number }) {
  // OpenRouter logo from lobe-icons (https://github.com/lobehub/lobe-icons), MIT License.
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      width={size}
      height={size}
      xmlns="http://www.w3.org/2000/svg"
    >
      <path fillRule="evenodd" d="M16.804 1.957l7.22 4.105v.087L16.73 10.21l.017-2.117-.821-.03c-1.059-.028-1.611.002-2.268.11-1.064.175-2.038.577-3.147 1.352L8.345 11.03c-.284.195-.495.336-.68.455l-.515.322-.397.234.385.23.53.338c.476.314 1.17.796 2.701 1.866 1.11.775 2.083 1.177 3.147 1.352l.3.045c.694.091 1.375.094 2.825.033l.022-2.159 7.22 4.105v.087L16.589 22l.014-1.862-.635.022c-1.386.042-2.137.002-3.138-.162-1.694-.28-3.26-.926-4.881-2.059l-2.158-1.5a21.997 21.997 0 00-.755-.498l-.467-.28a55.927 55.927 0 00-.76-.43C2.908 14.73.563 14.116 0 14.116V9.888l.14.004c.564-.007 2.91-.622 3.809-1.124l1.016-.58.438-.274c.428-.28 1.072-.726 2.686-1.853 1.621-1.133 3.186-1.78 4.881-2.059 1.152-.19 1.974-.213 3.814-.138l.02-1.907z" />
    </svg>
  );
}

function AnthropicIcon({ size }: { size: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
    >
      <path
        d="M17.304 3h-3.437l5.73 18h3.437L17.304 3zM6.696 3l-5.73 18H4.43l1.307-4.26h5.905L12.95 21h3.466L10.696 3H6.696zm.64 10.74L9.2 7.895l1.864 5.845H7.336z"
        fill="currentColor"
      />
    </svg>
  );
}

function GeminiIcon({ size }: { size: number }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      fill="currentColor"
      width={size}
      height={size}
    >
      <path d="M12 0C12 6.627 6.627 12 0 12c6.627 0 12 5.373 12 12 0-6.627 5.373-12 12-12-6.627 0-12-5.373-12-12z" />
    </svg>
  );
}

function AzureOpenAiIcon({ size }: { size: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
    >
      <path
        d="M13.05 4.24L6.56 18.05L2 18.22L7.68 7.32L13.05 4.24ZM14.15 5.56L16.65 10.25L12.38 18.04L22 18.25L14.15 5.56Z"
        fill="currentColor"
      />
    </svg>
  );
}

function AwsBedrockIcon({ size }: { size: number }) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
    >
      <path
        d="M12 2L2 7l10 5 10-5-10-5zM2 17l10 5 10-5M2 12l10 5 10-5"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function MicrosoftIcon({ size }: { size: number }) {
  // Microsoft's four-square mark, rendered monochrome with a gap.
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path d="M2 2h9.2v9.2H2zM12.8 2H22v9.2h-9.2zM2 12.8h9.2V22H2zM12.8 12.8H22V22h-9.2z" />
    </svg>
  );
}

function FireworksIcon({ size }: { size: number }) {
  // Stylized firework burst rendered monochrome.
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      xmlns="http://www.w3.org/2000/svg"
    >
      <circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none" />
      <path d="M12 2.5V6M12 18v3.5M2.5 12H6M18 12h3.5M5.2 5.2l2.5 2.5M16.3 16.3l2.5 2.5M18.8 5.2l-2.5 2.5M7.7 16.3l-2.5 2.5" />
    </svg>
  );
}

const PROVIDER_ICON_COMPONENTS: Record<DriverId, React.ComponentType<{ size: number }>> = {
  openai: OpenAiIcon,
  openrouter: OpenRouterIcon,
  azure_openai: AzureOpenAiIcon,
  openai_completions: OpenAiIcon,
  anthropic: AnthropicIcon,
  gemini: GeminiIcon,
  bedrock: AwsBedrockIcon,
  mai: MicrosoftIcon,
  fireworks: FireworksIcon,
};

const PROVIDER_LABELS: Record<DriverId, string> = {
  openai: "OpenAI (Responses)",
  openrouter: "OpenRouter",
  azure_openai: "Azure OpenAI",
  openai_completions: "OpenAI (Completions)",
  anthropic: "Anthropic",
  gemini: "Google Gemini",
  bedrock: "AWS Bedrock",
  mai: "Microsoft MAI",
  fireworks: "Fireworks AI",
};

// Short, accurate one-line taglines shown next to each provider in pickers.
const PROVIDER_DESCRIPTIONS: Record<DriverId, string> = {
  openai: "GPT and o-series models via the Responses API.",
  openrouter: "One key for a large multi-vendor model catalog.",
  azure_openai: "OpenAI models deployed in your Azure resource.",
  openai_completions: "OpenAI-compatible Chat Completions endpoints.",
  anthropic: "Claude models with extended thinking.",
  gemini: "Google Gemini models with context caching.",
  bedrock: "Models hosted on Amazon Bedrock.",
  mai: "Microsoft MAI models via Azure AI Foundry.",
  fireworks: "Fast, low-cost inference for open models.",
};

interface ProviderIconProps {
  providerType: DriverId;
  size?: "sm" | "md" | "lg";
  className?: string;
  showBackground?: boolean;
}

const sizeMap = {
  sm: { icon: 16, container: "p-1.5" },
  md: { icon: 20, container: "p-2" },
  lg: { icon: 24, container: "p-2.5" },
};

export function ProviderIcon({
  providerType,
  size = "md",
  className,
  showBackground = true,
}: ProviderIconProps) {
  const IconComponent = PROVIDER_ICON_COMPONENTS[providerType];
  const label = PROVIDER_LABELS[providerType];
  const { icon: iconSize, container } = sizeMap[size];

  if (!IconComponent) {
    return (
      <div className={cn(showBackground && "bg-primary/10", container, className)}>
        <Server className="text-primary" style={{ width: iconSize, height: iconSize }} />
      </div>
    );
  }

  return (
    <div className={cn(showBackground && "bg-primary/10", container, className)} title={label}>
      <IconComponent size={iconSize} />
    </div>
  );
}

export function getProviderLabel(providerType: DriverId): string {
  return PROVIDER_LABELS[providerType] || providerType;
}

export function getProviderDescription(providerType: DriverId): string {
  return PROVIDER_DESCRIPTIONS[providerType] || "";
}
