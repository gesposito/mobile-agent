import * as Location from 'expo-location';
import { Alert, StyleSheet, Text, View } from 'react-native';

import { FixtureButton } from '../FixtureButton';
import { colors } from '../theme';

export type PermissionText = 'Permission: unknown' | 'Permission: granted' | 'Permission: denied';

export const INITIAL_PERMISSION_TEXT: PermissionText = 'Permission: unknown';

type Props = {
  status: PermissionText;
  onStatusChange: (status: PermissionText) => void;
};

function showAlert() {
  Alert.alert('In-app alert', 'This alert is presented by the app.', [{ text: 'OK' }]);
}

// The status lives in the root, so it survives tab switches, and a request that resolves after
// the user has left the tab still updates it. It reads "unknown" until a request resolves; the
// screen never reads the current authorization on mount, so the launch text does not depend on
// earlier runs.
export function SystemScreen({ status, onStatusChange }: Props) {
  const requestPermission = () => {
    Location.requestForegroundPermissionsAsync()
      .then(({ status: result }) => {
        if (result === Location.PermissionStatus.GRANTED) {
          onStatusChange('Permission: granted');
        } else if (result === Location.PermissionStatus.DENIED) {
          onStatusChange('Permission: denied');
        }
      })
      .catch((error: unknown) => {
        console.warn('Location permission request failed', error);
      });
  };

  return (
    <View style={styles.container}>
      <FixtureButton
        testID="system-permission"
        label="Request location permission"
        onPress={requestPermission}
      />
      <Text testID="system-permission-status" style={styles.status}>
        {status}
      </Text>
      <FixtureButton testID="system-alert" label="Show in-app alert" onPress={showAlert} />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    padding: 24,
    gap: 12,
  },
  status: {
    fontSize: 17,
    color: colors.text,
  },
});
