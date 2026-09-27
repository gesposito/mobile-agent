import { useRef, useState } from 'react';
import { StatusBar, StyleSheet, View } from 'react-native';
import { initialWindowMetrics, SafeAreaProvider, SafeAreaView } from 'react-native-safe-area-context';

import { TabBar, type TabKey, tabFromLaunchOption } from './src/TabBar';
import { AnimScreen } from './src/screens/AnimScreen';
import { BigScreen } from './src/screens/BigScreen';
import { ListScreen } from './src/screens/ListScreen';
import { INITIAL_LOGIN_STATE, type LoginState, LoginScreen } from './src/screens/LoginScreen';
import { INITIAL_PERMISSION_TEXT, type PermissionText, SystemScreen } from './src/screens/SystemScreen';
import { colors } from './src/theme';

type Props = {
  // The FIXTURE_INITIAL_TAB launch option, passed by the native entry points that
  // plugins/withInitialTab.js generates.
  initialTab?: string;
};

// Only the selected screen is mounted, so the other tabs are absent from the accessibility
// tree. Screen state lives here, above that switch, so it survives tab switches. The list's
// scroll offset is a ref: it changes on every scroll frame and is read only when the list mounts.
//
// The SafeAreaView pads the screen and the tab bar by the system insets: the status bar and the
// home indicator on iOS, the status bar, the navigation or gesture bar and any display cutout on
// Android, where Expo draws edge to edge and React Native's own SafeAreaView is a plain View.
// Without initialMetrics the provider renders nothing until the first native insets event.
export default function App({ initialTab }: Props) {
  const [tab, setTab] = useState<TabKey>(() => tabFromLaunchOption(initialTab));
  const [login, setLogin] = useState<LoginState>(INITIAL_LOGIN_STATE);
  const [openRow, setOpenRow] = useState<number | null>(null);
  const listScrollOffset = useRef(0);
  const [animating, setAnimating] = useState(true);
  const [permission, setPermission] = useState<PermissionText>(INITIAL_PERMISSION_TEXT);

  return (
    <SafeAreaProvider initialMetrics={initialWindowMetrics}>
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
    </SafeAreaProvider>
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
