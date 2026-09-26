import * as Location from 'expo-location';
import { useState } from 'react';
import { Alert, StyleSheet, Text, View } from 'react-native';

import { FixtureButton } from '../FixtureButton';
import { colors } from '../theme';

type PermissionText = 'Permission: unknown' | 'Permission: granted' | 'Permission: denied';

function showAlert() {
  Alert.alert('In-app alert', 'This alert is presented by the app.', [{ text: 'OK' }]);
}

// The status reads "unknown" until a request resolves. The screen never reads the current
// authorization on mount, so its initial text does not depend on earlier runs.
export function SystemScreen() {
  const [status, setStatus] = useState<PermissionText>('Permission: unknown');

  const requestPermission = () => {
    Location.requestForegroundPermissionsAsync()
      .then(({ status: result }) => {
        if (result === Location.PermissionStatus.GRANTED) {
          setStatus('Permission: granted');
        } else if (result === Location.PermissionStatus.DENIED) {
          setStatus('Permission: denied');
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
