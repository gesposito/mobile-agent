import { useCallback, useState } from 'react';
import { FlatList, type ListRenderItem, Pressable, StyleSheet, Text, View } from 'react-native';

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

// The detail view is drawn over the list instead of replacing it, so the list keeps its
// scroll position; while it is open the list is hidden from accessibility.
export function ListScreen() {
  const [selected, setSelected] = useState<number | null>(null);
  const detailOpen = selected !== null;

  const renderItem = useCallback<ListRenderItem<number>>(
    ({ item }) => <Row n={item} onPress={setSelected} />,
    [],
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
        />
      </View>
      {selected !== null && <Detail n={selected} onBack={() => setSelected(null)} />}
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
