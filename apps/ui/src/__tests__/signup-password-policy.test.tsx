import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import SignupPage from "@/app/(auth)/signup/page";

// EVE-719: a short-but-nonempty password used to be swallowed by the password
// input's native `minLength`, so the calm inline policy error never rendered.
// These tests assert the JS-driven inline `role="alert"` message is reachable
// and that submission is blocked (the register mutation is never called).

const mockRegister = jest.fn();

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: jest.fn() }),
  useSearchParams: () => new URLSearchParams(""),
}));

jest.mock("@/hooks/use-auth", () => ({
  useAuthConfig: () => ({
    data: {
      signup_enabled: true,
      password_auth_enabled: true,
      oauth_providers: [],
      signup_email_confirm: false,
      captcha: null,
    },
    isLoading: false,
  }),
  useRegister: () => ({ mutateAsync: mockRegister, isPending: false }),
}));

jest.mock("@/hooks", () => ({ usePageTitle: jest.fn() }));
jest.mock("@/lib/api/auth", () => ({ getOAuthUrl: jest.fn(() => "/oauth") }));
jest.mock("@/lib/auth-redirect", () => ({
  buildLoginHref: () => "/login",
  consumeReturnTo: () => null,
  isBackendNavigationPath: () => false,
  persistReturnTo: jest.fn(),
  sanitizeReturnTo: () => "",
}));
jest.mock("@/components/auth/turnstile-widget", () => ({
  TurnstileWidget: () => null,
}));

const POLICY_MESSAGE = /at least 12 characters including a number/i;

function fillAndSubmit(password: string) {
  fireEvent.change(screen.getByLabelText("Email"), {
    target: { value: "user@example.com" },
  });
  fireEvent.change(screen.getByLabelText("Password"), {
    target: { value: password },
  });
  fireEvent.click(screen.getByRole("button", { name: /create account/i }));
}

describe("signup password policy inline error", () => {
  beforeEach(() => {
    mockRegister.mockReset();
    mockRegister.mockResolvedValue(undefined);
  });

  it("shows the inline alert for a short password and blocks submit", async () => {
    render(<SignupPage />);
    fillAndSubmit("abc1");

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(POLICY_MESSAGE);
    expect(mockRegister).not.toHaveBeenCalled();
  });

  it("shows the inline alert for a long password missing a digit and blocks submit", async () => {
    render(<SignupPage />);
    fillAndSubmit("abcdefghijklmno");

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(POLICY_MESSAGE);
    expect(mockRegister).not.toHaveBeenCalled();
  });

  it("submits a compliant password (length + digit)", async () => {
    render(<SignupPage />);
    fillAndSubmit("abcdefghijk1");

    await waitFor(() => expect(mockRegister).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
