import { useRef, useState } from 'react';
import { LogBox, SafeAreaView, StatusBar, StyleSheet, View } from 'react-native';

import { TabBar, type TabKey } from './src/TabBar';
import { AnimScreen } from './src/screens/AnimScreen';
import { BigScreen } from './src/screens/BigScreen';
import { ListScreen } from './src/screens/ListScreen';
import { INITIAL_LOGIN_STATE, type LoginState, LoginScreen } from './src/screens/LoginScreen';
import { INITIAL_PERMISSION_TEXT, type PermissionText, SystemScreen } from './src/screens/SystemScreen';
import { colors } from './src/theme';

// React Native 0.86 marks its built-in SafeAreaView deprecated. The fixture keeps it so the
// runtime dependencies stay React Native plus expo-location; silencing the dev-only LogBox
// notice keeps Debug screenshots identical to Release ones.
LogBox.ignoreLogs(['SafeAreaView has been deprecated']);

// Only the selected screen is mounted, so the other tabs are absent from the accessibility
// tree. Screen state lives here, above that switch, so it survives tab switches. The list's
// scroll offset is a ref: it changes on every scroll frame and is read only when the list mounts.
export default function App() {
  const [tab, setTab] = useState<TabKey>('login');
  const [login, setLogin] = useState<LoginState>(INITIAL_LOGIN_STATE);
  const [openRow, setOpenRow] = useState<number | null>(null);
  const listScrollOffset = useRef(0);
  const [animating, setAnimating] = useState(true);
  const [permission, setPermission] = useState<PermissionText>(INITIAL_PERMISSION_TEXT);

  return (
    <SafeAreaView style={styles.root}>
      <StatusBar barStyle="dark-content" />
      <View style={styles.screen}>
        {tab === 'login' && <LoginScreen state={login} onChange={setLogin} />}
        {tab === 'list' && (
          <ListScreen openRow={openRow} onOpenRow={setOpenRow} scrollOffset={listScrollOffset} />
        )}
        {tab === 'big' && <BigScreen />}
        {tab === 'anim' && <AnimScreen running={animating} onRunningChange={setAnimating} />}
        {tab === 'system' && <SystemScreen status={permission} onStatusChange={setPermission} />}
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
