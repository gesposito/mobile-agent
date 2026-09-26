import { useState } from 'react';
import { Keyboard, StyleSheet, Text, TextInput, View } from 'react-native';

import { FixtureButton } from '../FixtureButton';
import { colors } from '../theme';

export const VALID_PASSWORD = 'hunter2';
const SIGNED_OUT = 'Signed out';

// The inputs are uncontrolled so fast programmatic typing never races a JS re-render;
// onChangeText only mirrors the native text into state for the submit handler.
export function LoginScreen() {
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [status, setStatus] = useState(SIGNED_OUT);

  const submit = () => {
    Keyboard.dismiss();
    setStatus(password === VALID_PASSWORD ? `Welcome, ${username}` : 'Invalid credentials');
  };

  return (
    <View style={styles.container}>
      <TextInput
        testID="login-username"
        accessibilityLabel="Username"
        placeholder="Username"
        onChangeText={setUsername}
        autoCapitalize="none"
        autoCorrect={false}
        spellCheck={false}
        autoComplete="off"
        textContentType="none"
        importantForAutofill="no"
        style={styles.input}
      />
      <TextInput
        testID="login-password"
        accessibilityLabel="Password"
        placeholder="Password"
        onChangeText={setPassword}
        secureTextEntry
        autoCapitalize="none"
        autoCorrect={false}
        spellCheck={false}
        autoComplete="off"
        textContentType="none"
        importantForAutofill="no"
        style={styles.input}
      />
      <FixtureButton testID="login-submit" label="Sign in" onPress={submit} />
      <Text testID="login-status" style={styles.status}>
        {status}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    padding: 24,
    gap: 12,
  },
  input: {
    height: 44,
    paddingHorizontal: 12,
    borderRadius: 10,
    backgroundColor: colors.field,
    color: colors.text,
    fontSize: 17,
  },
  status: {
    marginTop: 8,
    fontSize: 17,
    color: colors.text,
  },
});
