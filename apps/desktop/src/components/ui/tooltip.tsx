import * as React from "react"
import { Tooltip as TooltipPrimitive } from "radix-ui"

import { cn } from "@/lib/utils"

/**
 * How long a pointer rests on a trigger before its tooltip opens. Not 0: an
 * instant tooltip flashes on every pointer pass across a toolbar and reads
 * as noise; 200 ms is below what registers as a wait. Well under the
 * browser's fixed ~1 s native `title` delay this replaces (#376).
 */
export const TOOLTIP_DELAY_MS = 200

/**
 * After one tooltip has shown, sibling tooltips under the same provider open
 * with no delay for this long — the "sweep across the toolbar" behaviour.
 */
export const TOOLTIP_SKIP_DELAY_MS = 300

// Radix throws for a `Tooltip` rendered outside a `TooltipProvider`. The app
// mounts one provider at its root so every tooltip shares the skip-delay
// state; a `Tooltip` mounted anywhere else (a component test, an isolated
// render) gets a provider of its own instead of a crash.
const HasTooltipProvider = React.createContext(false)

function TooltipProvider({
  delayDuration = TOOLTIP_DELAY_MS,
  skipDelayDuration = TOOLTIP_SKIP_DELAY_MS,
  children,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Provider>) {
  return (
    <HasTooltipProvider.Provider value={true}>
      <TooltipPrimitive.Provider
        data-slot="tooltip-provider"
        delayDuration={delayDuration}
        skipDelayDuration={skipDelayDuration}
        {...props}
      >
        {children}
      </TooltipPrimitive.Provider>
    </HasTooltipProvider.Provider>
  )
}

function Tooltip({
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Root>) {
  const hasProvider = React.useContext(HasTooltipProvider)
  const root = <TooltipPrimitive.Root data-slot="tooltip" {...props} />
  return hasProvider ? root : <TooltipProvider>{root}</TooltipProvider>
}

function TooltipTrigger({
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Trigger>) {
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" {...props} />
}

// Styled like the native `title` bubble it replaces (light popover surface,
// hairline border, small dark text, no arrow) so only the delay changes for
// the reader; shadcn's dark pill with an arrow would read as a new control.
function TooltipContent({
  className,
  sideOffset = 4,
  children,
  ...props
}: React.ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        data-slot="tooltip-content"
        sideOffset={sideOffset}
        className={cn(
          "z-50 w-fit max-w-xs rounded-md border border-border bg-popover px-2 py-1 text-xs text-popover-foreground shadow-sm duration-100 data-open:animate-in data-open:fade-in-0 data-closed:animate-out data-closed:fade-out-0",
          className
        )}
        {...props}
      >
        {children}
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  )
}

export { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger }
