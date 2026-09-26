import * as Location from 'expo-location';
import { Alert, StyleSheet, View } from 'react-native';

import { FixtureButton } from '../FixtureButton';

function requestLocationPermission() {
  Location.requestForegroundPermissionsAsync().catch((error: unknown) => {
    console.warn('Location permission request failed', error);
  });
}

function showAlert() {
  Alert.alert('Fixture alert', 'An in-app alert with a single OK button.', [{ text: 'OK' }]);
}

export function SystemScreen() {
  return (
    <View style={styles.container}>
      <FixtureButton
        testID="system-permission"
        label="Request location permission"
        onPress={requestLocationPermission}
      />
      <FixtureButton testID="system-alert" label="Show alert" onPress={showAlert} />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    padding: 24,
    gap: 12,
  },
});
