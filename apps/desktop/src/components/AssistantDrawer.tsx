import React from "react";
import { Drawer } from "../ui/Drawer";
import { AssistantConversation, type AssistantContext } from "./AssistantConversation";

export type { AssistantContext };

/**
 * Right-hand chat drawer: docks the shared `AssistantConversation` (see there
 * for the conversation behavior) with the resource/namespace the user had
 * open passed through as `context`.
 */
export function AssistantDrawer({
  open,
  onClose,
  context,
}: {
  open: boolean;
  onClose: () => void;
  context?: AssistantContext;
}) {
  return (
    <Drawer open={open} onClose={onClose} title="Assistant" defaultWidth={420}>
      <AssistantConversation context={context} />
    </Drawer>
  );
}
