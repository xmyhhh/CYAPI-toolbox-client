import { Workbench } from '../features/workflows/Workbench';
import type { CreationSection } from '../lib/workbenches';
import type { ThemePreference } from '../lib/theme';

/**
 * The user workbench is a native React page. Only the ComfyUI canvas inside the
 * editor dialog is loaded from the workflow service; there is no remote page
 * or native child WebView involved.
 */
export function Workflows({
  initialSection,
  navigation,
  onSectionChange,
  onLockedChange,
  theme,
  visible,
}: {
  initialSection: CreationSection;
  navigation?: { section: CreationSection; id: number };
  onSectionChange: (section: CreationSection) => void;
  onLockedChange: (locked: boolean) => void;
  theme: ThemePreference;
  visible: boolean;
}) {
  return (
    <Workbench
      initialSection={initialSection}
      navigation={navigation}
      onSectionChange={onSectionChange}
      onLockedChange={onLockedChange}
      theme={theme}
      visible={visible}
    />
  );
}
