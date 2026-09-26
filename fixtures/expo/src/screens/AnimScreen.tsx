import { useEffect, useMemo, useState } from 'react';
import { Animated, Easing, StyleSheet, Switch, Text, useAnimatedValue, View } from 'react-native';

import { colors } from '../theme';

const REVOLUTION_MS = 1000;

// The loop runs on the native driver. Turning the switch off stops it and resets the
// spinner to 0 degrees, so the stopped state always renders the same frame.
export function AnimScreen() {
  const [running, setRunning] = useState(true);
  const rotation = useAnimatedValue(0);
  const rotate = useMemo(
    () => rotation.interpolate({ inputRange: [0, 1], outputRange: ['0deg', '360deg'] }),
    [rotation],
  );

  useEffect(() => {
    if (!running) {
      return undefined;
    }
    const loop = Animated.loop(
      Animated.timing(rotation, {
        toValue: 1,
        duration: REVOLUTION_MS,
        easing: Easing.linear,
        useNativeDriver: true,
        isInteraction: false,
      }),
    );
    loop.start();
    return () => {
      loop.stop();
      rotation.setValue(0);
    };
  }, [running, rotation]);

  return (
    <View style={styles.container}>
      <Animated.View
        testID="anim-spinner"
        accessible
        accessibilityRole="image"
        accessibilityLabel="Spinner"
        style={[styles.spinner, { transform: [{ rotate }] }]}
      >
        <View style={styles.mark} />
      </Animated.View>
      <View style={styles.toggleRow}>
        <Text accessible={false} style={styles.toggleLabel}>
          Animate
        </Text>
        <Switch
          testID="anim-toggle"
          accessibilityLabel="Animate"
          value={running}
          onValueChange={setRunning}
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    gap: 48,
  },
  spinner: {
    width: 120,
    height: 120,
    borderRadius: 24,
    backgroundColor: colors.tint,
    alignItems: 'center',
    paddingTop: 12,
  },
  mark: {
    width: 12,
    height: 36,
    borderRadius: 6,
    backgroundColor: colors.onTint,
  },
  toggleRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 12,
  },
  toggleLabel: {
    fontSize: 17,
    color: colors.text,
  },
});
