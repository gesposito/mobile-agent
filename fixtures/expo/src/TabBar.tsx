import { Pressable, StyleSheet, Text, View } from 'react-native';

import { colors } from './theme';

export type TabKey = 'login' | 'list' | 'big' | 'anim' | 'system';

export const TABS: ReadonlyArray<{ key: TabKey; label: string }> = [
  { key: 'login', label: 'Login' },
  { key: 'list', label: 'List' },
  { key: 'big', label: 'Big' },
  { key: 'anim', label: 'Anim' },
  { key: 'system', label: 'System' },
];

// Maps the FIXTURE_INITIAL_TAB launch option to a tab; empty or unknown values select Login.
export function tabFromLaunchOption(value: string | undefined): TabKey {
  return TABS.find((tab) => tab.key === value)?.key ?? 'login';
}

type Props = {
  active: TabKey;
  onSelect: (key: TabKey) => void;
};

// Role "button" rather than "tab": React Native maps "tab" to no iOS trait, which would
// hide the items from interactive filters. The selected state adds the Selected trait.
export function TabBar({ active, onSelect }: Props) {
  return (
    <View style={styles.bar}>
      {TABS.map(({ key, label }) => {
        const selected = key === active;
        return (
          <Pressable
            key={key}
            testID={`tab-${key}`}
            accessibilityRole="button"
            accessibilityLabel={label}
            accessibilityState={{ selected }}
            onPress={() => onSelect(key)}
            style={styles.tab}
          >
            <Text style={[styles.label, selected && styles.selectedLabel]}>{label}</Text>
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  bar: {
    flexDirection: 'row',
    height: 56,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.separator,
    backgroundColor: colors.background,
  },
  tab: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
  },
  label: {
    fontSize: 15,
    fontWeight: '500',
    color: colors.inactive,
  },
  selectedLabel: {
    color: colors.tint,
    fontWeight: '700',
  },
});
