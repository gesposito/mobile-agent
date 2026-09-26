import { Pressable, StyleSheet, Text } from 'react-native';

import { colors } from './theme';

type Props = {
  testID: string;
  label: string;
  onPress: () => void;
};

export function FixtureButton({ testID, label, onPress }: Props) {
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={({ pressed }) => [styles.button, pressed && styles.pressed]}
    >
      <Text style={styles.label}>{label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  button: {
    minHeight: 44,
    paddingHorizontal: 16,
    borderRadius: 10,
    backgroundColor: colors.tint,
    alignItems: 'center',
    justifyContent: 'center',
  },
  pressed: {
    opacity: 0.6,
  },
  label: {
    color: colors.onTint,
    fontSize: 17,
    fontWeight: '600',
  },
});
