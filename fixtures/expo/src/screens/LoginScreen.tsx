import { type Dispatch, type SetStateAction, useState } from 'react';
import { Keyboard, StyleSheet, Text, TextInput, View } from 'react-native';

import { FixtureButton } from '../FixtureButton';
import { colors } from '../theme';

export const VALID_PASSWORD = 'hunter2';

export type LoginState = {
  username: string;
  password: string;
  status: string;
};

export const INITIAL_LOGIN_STATE: LoginState = {
  username: '',
  password: '',
  status: 'Signed out',
};

type Props = {
  state: LoginState;
  onChange: Dispatch<SetStateAction<LoginState>>;
};

// The inputs are uncontrolled so fast programmatic typing never races a JS re-render: each
// mount seeds them once from the stored text, and every edit is written back to the root.
// Updates are functional, so a submit right after the last keystroke sees that keystroke.
export function LoginScreen({ state, onChange }: Props) {
  const [initial] = useState(state);

  const submit = () => {
    Keyboard.dismiss();
    onChange((current) => ({
      ...current,
      status:
        current.password === VALID_PASSWORD
          ? `Welcome, ${current.username}`
          : 'Invalid credentials',
    }));
  };

  return (
    <View style={styles.container}>
      <TextInput
        testID="login-username"
        accessibilityLabel="Username"
        placeholder="Username"
        defaultValue={initial.username}
        onChangeText={(username) => onChange((current) => ({ ...current, username }))}
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
        defaultValue={initial.password}
        onChangeText={(password) => onChange((current) => ({ ...current, password }))}
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
        {state.status}
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
