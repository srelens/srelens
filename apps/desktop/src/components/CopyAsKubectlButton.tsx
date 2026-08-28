import React, { useState } from "react";
import { Copy, ScrollText } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Button } from "@/components/ui/button";
import { toKubectl } from "@srelens/core";
import { copyKubectlCommand } from "@srelens/core";

const MENU_ITEM_CLASS =
  "flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm hover:bg-accent";

/**
 * Header affordance for copying a resource's kubectl get/describe command.
 * Collapses what used to be two separate icon buttons into one, opening a
 * small menu — headers were getting crowded, and Copy/ScrollText don't
 * self-describe as get-vs-describe on their own.
 */
export function CopyAsKubectlButton({
  kind,
  name,
  namespace,
  context,
}: {
  kind: string;
  name: string;
  namespace?: string | null;
  context: string;
}) {
  const [open, setOpen] = useState(false);

  async function copy(action: "get" | "describe") {
    await copyKubectlCommand(
      toKubectl({ action, kind, name, namespace, context, output: action === "get" ? "yaml" : undefined }),
    );
    setOpen(false);
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button type="button" variant="ghost" size="icon-sm" aria-label="Copy as kubectl" title="Copy as kubectl">
          <Copy aria-hidden="true" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-44 p-1">
        <div role="group" aria-label="Copy as kubectl">
          <button type="button" className={MENU_ITEM_CLASS} onClick={() => void copy("get")}>
            <Copy className="h-4 w-4" aria-hidden="true" />
            Copy get
          </button>
          <button type="button" className={MENU_ITEM_CLASS} onClick={() => void copy("describe")}>
            <ScrollText className="h-4 w-4" aria-hidden="true" />
            Copy describe
          </button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
