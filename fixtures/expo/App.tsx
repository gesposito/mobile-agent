import { type ComponentType, useState } from 'react';
import { LogBox, SafeAreaView, StatusBar, StyleSheet, View } from 'react-native';

import { TabBar, type TabKey } from './src/TabBar';
import { AnimScreen } from './src/screens/AnimScreen';
import { BigScreen } from './src/screens/BigScreen';
import { ListScreen } from './src/screens/ListScreen';
import { LoginScreen } from './src/screens/LoginScreen';
import { SystemScreen } from './src/screens/SystemScreen';
import { colors } from './src/theme';

// React Native 0.86 marks its built-in SafeAreaView deprecated. The fixture keeps it so the
// runtime dependencies stay React Native plus expo-location; silencing the dev-only LogBox
// notice keeps Debug screenshots identical to Release ones.
LogBox.ignoreLogs(['SafeAreaView has been deprecated']);

const SCREENS: Record<TabKey, ComponentType> = {
  login: LoginScreen,
  list: ListScreen,
  big: BigScreen,
  anim: AnimScreen,
  system: SystemScreen,
};

// Only the selected screen is mounted; switching tabs unmounts the previous one, so every
// visit to a tab starts from the same initial state.
export default function App() {
  const [tab, setTab] = useState<TabKey>('login');
  const Screen = SCREENS[tab];

  return (
    <SafeAreaView style={styles.root}>
      <StatusBar barStyle="dark-content" />
      <View style={styles.screen}>
        <Screen />
      </View>
      <TabBar active={tab} onSelect={setTab} />
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
    backgroundColor: colors.background,
  },
  screen: {
    flex: 1,
  },
});
