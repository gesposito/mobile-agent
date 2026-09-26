import { Pressable, ScrollView, StyleSheet, Text, useWindowDimensions, View } from 'react-native';

import { colors } from '../theme';

export const CELL_COUNT = 500;
const COLUMNS = 20;
const PADDING = 8;
const CELLS: readonly number[] = Array.from({ length: CELL_COUNT }, (_, index) => index + 1);

// Every cell mounts in one pass (a ScrollView, never a virtualized list). Cell size follows
// the window width so a 402 pt wide screen shows the whole 20 x 25 grid without scrolling.
export function BigScreen() {
  const { width } = useWindowDimensions();
  const size = Math.floor((width - 2 * PADDING) / COLUMNS);
  const cellSize = { width: size, height: size };

  return (
    <View style={styles.fill}>
      <Text testID="big-count" accessibilityRole="header" style={styles.header}>
        {`${CELL_COUNT} elements`}
      </Text>
      <ScrollView contentContainerStyle={styles.content}>
        <View style={[styles.grid, { width: size * COLUMNS }]}>
          {CELLS.map((n) => (
            <Pressable
              key={n}
              testID={`cell-${n}`}
              accessible
              accessibilityRole="button"
              accessibilityLabel={`Cell ${n}`}
              style={({ pressed }) => [styles.cell, cellSize, pressed && styles.cellPressed]}
            >
              <Text allowFontScaling={false} numberOfLines={1} style={styles.cellText}>
                {n}
              </Text>
            </Pressable>
          ))}
        </View>
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: {
    flex: 1,
  },
  header: {
    paddingHorizontal: 16,
    paddingVertical: 12,
    fontSize: 17,
    fontWeight: '600',
    color: colors.text,
  },
  content: {
    padding: PADDING,
    alignItems: 'center',
  },
  grid: {
    flexDirection: 'row',
    flexWrap: 'wrap',
  },
  cell: {
    alignItems: 'center',
    justifyContent: 'center',
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.cellBorder,
    backgroundColor: colors.cell,
  },
  cellPressed: {
    backgroundColor: colors.tint,
  },
  cellText: {
    fontSize: 7,
    color: colors.text,
  },
});
