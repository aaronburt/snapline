import type { IconName } from '../lib/icons';

export interface StepItem {
  id: string;
  label: string;
  icon: IconName;
  checked?: boolean;
  note?: string;
  removable?: boolean;
}
