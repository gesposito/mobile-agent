import { useCallback, useState } from 'react';
import {
  Dimensions,
  type LayoutChangeEvent,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from 'react-native';

import { colors } from '../theme';

export const CELL_COUNT = 500;
// Columns when the grid's area is taller than wide (20 x 25) and when it is wider (25 x 20).
const PORTRAIT_COLUMNS = 20;
const LANDSCAPE_COLUMNS = 25;
const PADDING = 8;
const CELLS: readonly number[] = Array.from({ length: CELL_COUNT }, (_, index) => index + 1);

type Grid = { columns: number; size: number };

// The largest whole cell size at which the grid fits `area`.
function fitGrid(area: { width: number; height: number }): Grid {
  const columns = area.width > area.height ? LANDSCAPE_COLUMNS : PORTRAIT_COLUMNS;
  const rows = CELL_COUNT / columns;
  const size = Math.floor(
    Math.min((area.width - 2 * PADDING) / columns, (area.height - 2 * PADDING) / rows),
  );
  return { columns, size: Math.max(1, size) };
}

// Every cell mounts in one pass (a ScrollView, never a virtualized list). The grid fits the
// ScrollView, so it is all on screen in either orientation and tools that drop off-screen nodes
// still see all 500 cells: 19 pt cells on a 402 pt wide iPhone in portrait. The window stands
// in for the ScrollView until its first layout; in portrait both give the same grid, and
// returning the current state then skips re-rendering the 500 cells.
export function BigScreen() {
  const [{ columns, size }, setGrid] = useState(() => fitGrid(Dimensions.get('window')));
  const cellSize = { width: size, height: size };

  const fitToViewport = useCallback(({ nativeEvent: { layout } }: LayoutChangeEvent) => {
    const next = fitGrid(layout);
    setGrid((current) =>
      current.columns === next.columns && current.size === next.size ? current : next,
    );
  }, []);

  return (
    <View style={styles.fill}>
      <Text testID="big-count" accessibilityRole="header" style={styles.header}>
        {`${CELL_COUNT} elements`}
      </Text>
      <ScrollView contentContainerStyle={styles.content} onLayout={fitToViewport}>
        <View style={[styles.grid, { width: size * columns }]}>
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
