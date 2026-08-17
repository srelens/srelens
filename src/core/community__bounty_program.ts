export interface ValidationContext {
  isValid: boolean;
  isExpired?: boolean;
  timestamp?: number;
}

export function validateAndProcess(item?: ValidationContext | null): boolean {
  if (!item || typeof item !== 'object') {
    return false;
  }
  if (!item.isValid || item.isExpired) {
    return false;
  }
  return true;
}