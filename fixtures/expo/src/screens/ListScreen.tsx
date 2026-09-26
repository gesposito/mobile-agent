import { type RefObject, useCallback, useState } from 'react';
import {
  FlatList,
  type ListRenderItem,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
  Pressable,
  StyleSheet,
  Text,
  View,
} from 'react-native';

import { FixtureButton } from '../FixtureButton';
import { colors } from '../theme';

export const ROW_COUNT = 200;
const ROW_HEIGHT = 48;
const ROWS: readonly number[] = Array.from({ length: ROW_COUNT }, (_, index) => index + 1);

const keyExtractor = (n: number) => String(n);
const getItemLayout = (_data: ArrayLike<number> | null | undefined, index: number) => ({
  length: ROW_HEIGHT,
  offset: ROW_HEIGHT * index,
  index,
});

type Props = {
  openRow: number | null;
  onOpenRow: (n: number | null) => void;
  scrollOffset: RefObject<number>;
};

// The open row and the scroll offset live in the root, so both survive tab switches. The list
// remounts on every visit and restores the offset exactly: initialScrollIndex makes the first
// render mount the rows at that offset, and contentOffset places the scroll view there
// (FlatList skips its own scroll to the index when contentOffset is set). The detail view is
// drawn over the list, which is hidden from accessibility while the detail is open.
export function ListScreen({ openRow, onOpenRow, scrollOffset }: Props) {
  const [restore] = useState(() => ({
    initialScrollIndex: Math.floor(scrollOffset.current / ROW_HEIGHT),
    contentOffset: { x: 0, y: scrollOffset.current },
  }));
  const detailOpen = openRow !== null;

  // Clamped to the scrollable range so a tab switch during a bounce stores a resting offset.
  // Events that arrive before the content is laid out carry no usable range and are skipped.
  const saveOffset = useCallback(
    ({ nativeEvent }: NativeSyntheticEvent<NativeScrollEvent>) => {
      const maxOffset = nativeEvent.contentSize.height - nativeEvent.layoutMeasurement.height;
      if (maxOffset > 0) {
        scrollOffset.current = Math.min(Math.max(0, nativeEvent.contentOffset.y), maxOffset);
      }
    },
    [scrollOffset],
  );

  const renderItem = useCallback<ListRenderItem<number>>(
    ({ item }) => <Row n={item} onPress={onOpenRow} />,
    [onOpenRow],
  );

  return (
    <View style={styles.fill}>
      <View
        style={styles.fill}
        accessibilityElementsHidden={detailOpen}
        importantForAccessibility={detailOpen ? 'no-hide-descendants' : 'auto'}
      >
        <FlatList
          data={ROWS}
          renderItem={renderItem}
          keyExtractor={keyExtractor}
          getItemLayout={getItemLayout}
          initialNumToRender={20}
          windowSize={5}
          initialScrollIndex={restore.initialScrollIndex}
          contentOffset={restore.contentOffset}
          scrollEventThrottle={16}
          onScroll={saveOffset}
          onScrollEndDrag={saveOffset}
          onMomentumScrollEnd={saveOffset}
        />
      </View>
      {openRow !== null && <Detail n={openRow} onBack={() => onOpenRow(null)} />}
    </View>
  );
}

function Row({ n, onPress }: { n: number; onPress: (n: number) => void }) {
  return (
    <Pressable
      testID={`row-${n}`}
      accessibilityRole="button"
      accessibilityLabel={`Row ${n}`}
      onPress={() => onPress(n)}
      style={({ pressed }) => [styles.row, pressed && styles.rowPressed]}
    >
      <Text style={styles.rowText}>Row {n}</Text>
      <Text style={styles.chevron}>›</Text>
    </Pressable>
  );
}

function Detail({ n, onBack }: { n: number; onBack: () => void }) {
  return (
    <View style={styles.detail}>
      <View style={styles.backRow}>
        <FixtureButton testID="detail-back" label="Back" onPress={onBack} />
      </View>
      <Text testID={`detail-${n}`} style={styles.detailTitle}>
        {`Detail ${n}`}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  fill: {
    flex: 1,
  },
  row: {
    height: ROW_HEIGHT,
    paddingHorizontal: 16,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.separator,
    backgroundColor: colors.background,
  },
  rowPressed: {
    backgroundColor: colors.pressed,
  },
  rowText: {
    fontSize: 17,
    color: colors.text,
  },
  chevron: {
    fontSize: 22,
    color: colors.inactive,
  },
  detail: {
    position: 'absolute',
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
    padding: 24,
    gap: 12,
    backgroundColor: colors.background,
  },
  backRow: {
    flexDirection: 'row',
  },
  detailTitle: {
    fontSize: 28,
    fontWeight: '700',
    color: colors.text,
  },
});
