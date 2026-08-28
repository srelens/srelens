import srelensMark from "../assets/srelens-mark.svg";
import { Button } from "@/components/ui/button";
import { devLogin, loginUrl } from "@srelens/core";

/**
 * Pre-auth gate shown on web when no session cookie is present. Mirrors
 * LandingPage's brand mark + shadcn Button conventions (no bespoke colors).
 */
export function LoginScreen() {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-6 bg-background px-4 text-center">
      <img src={srelensMark} alt="" className="size-14" />
      <div className="flex flex-col gap-1.5">
        <h1 className="text-xl font-semibold text-foreground">Sign in to srelens</h1>
        <p className="text-sm text-muted-foreground">Continue with your organization&apos;s single sign-on.</p>
      </div>
      <Button asChild>
        <a href={loginUrl}>Sign in with SSO</a>
      </Button>
      <Button
        variant="ghost"
        size="sm"
        onClick={() => void devLogin().then(() => location.reload()).catch((err) => alert(String(err)))}
      >
        Developer login
      </Button>
    </div>
  );
}
