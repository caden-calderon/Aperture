import type { Block, Role } from "$lib/types";

export type ThreadPosition = "first" | "middle" | "last";

const THREAD_ROLES = new Set<Role>(["user", "assistant", "tool_use", "tool_result"]);

function canContinueThread(current: Block, next: Block | undefined): boolean {
  if (!next) return false;
  if (!THREAD_ROLES.has(current.role) || !THREAD_ROLES.has(next.role)) return false;

  const turnDelta = next.metadata.turnIndex - current.metadata.turnIndex;
  if (turnDelta < 0 || turnDelta > 1) return false;

  switch (current.role) {
    case "user":
      return next.role === "assistant" || next.role === "tool_use" || next.role === "tool_result";
    case "assistant":
      return next.role === "assistant" || next.role === "tool_use" || next.role === "tool_result";
    case "tool_use":
      return next.role === "tool_result" || next.role === "tool_use" || next.role === "assistant";
    case "tool_result":
      return next.role === "tool_result" || next.role === "tool_use" || next.role === "assistant";
    default:
      return false;
  }
}

export function computeThreadPositions(blocks: Block[]): Map<string, ThreadPosition> {
  const positions = new Map<string, ThreadPosition>();
  let groupStart = 0;

  for (let i = 0; i < blocks.length; i++) {
    const current = blocks[i];
    const next = blocks[i + 1];

    if (canContinueThread(current, next)) {
      continue;
    }

    const groupEnd = i;
    const groupLength = groupEnd - groupStart + 1;
    if (groupLength > 1) {
      for (let idx = groupStart; idx <= groupEnd; idx++) {
        const blockId = blocks[idx].id;
        if (idx === groupStart) {
          positions.set(blockId, "first");
        } else if (idx === groupEnd) {
          positions.set(blockId, "last");
        } else {
          positions.set(blockId, "middle");
        }
      }
    }

    groupStart = i + 1;
  }

  return positions;
}
