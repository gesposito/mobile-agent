package dev.mobileagent.fixture.compose

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.Color
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.platform.ViewConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Dark system bar icons in every system theme, matching the fixed light color scheme.
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT),
        )
        setContent { FixtureApp() }
    }
}

private enum class FixtureTab(val label: String, val tag: String) {
    LOGIN("Login", "tab-login"),
    LIST("List", "tab-list"),
    BIG("Big", "tab-big"),
    ANIM("Anim", "tab-anim"),
    SYSTEM("System", "tab-system"),
}

/**
 * Bottom tab bar over one screen per tab, Login first. Only the selected screen is composed, so the
 * other tabs contribute nothing to the semantics tree. The saveable state holder keeps each
 * screen's saveable state (typed text, statuses, scroll position, open detail, toggle) while its
 * tab is not selected and restores it when the tab returns.
 */
@Composable
private fun FixtureApp() {
    var tab by rememberSaveable { mutableStateOf(FixtureTab.LOGIN) }
    val screens = rememberSaveableStateHolder()
    val density = LocalDensity.current
    // Font scale is pinned so the device's font size setting cannot change the layout.
    CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale = 1f)) {
        MaterialTheme {
            Scaffold(
                // Exposes every testTag in this window as the node's resource-id to UI automation.
                modifier = Modifier.semantics { testTagsAsResourceId = true },
                bottomBar = { TabBar(selected = tab, onSelect = { tab = it }) },
            ) { padding ->
                Box(Modifier.fillMaxSize().padding(padding)) {
                    screens.SaveableStateProvider(key = tab.name) {
                        when (tab) {
                            FixtureTab.LOGIN -> LoginScreen()
                            FixtureTab.LIST -> ListScreen()
                            FixtureTab.BIG -> BigScreen()
                            FixtureTab.ANIM -> AnimScreen()
                            FixtureTab.SYSTEM -> SystemScreen()
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun TabBar(selected: FixtureTab, onSelect: (FixtureTab) -> Unit) {
    NavigationBar {
        FixtureTab.entries.forEach { tab ->
            NavigationBarItem(
                selected = tab == selected,
                onClick = { onSelect(tab) },
                icon = { TabDot() },
                label = { Text(tab.label) },
                modifier = Modifier.testTag(tab.tag),
            )
        }
    }
}

@Composable
private fun TabDot() {
    Box(Modifier.size(8.dp).background(LocalContentColor.current, CircleShape))
}

@Composable
private fun ScreenTitle(text: String, modifier: Modifier = Modifier) {
    Text(
        text = text,
        modifier = modifier.semantics { heading() },
        style = MaterialTheme.typography.headlineMedium,
    )
}

private const val VALID_PASSWORD = "hunter2"

/**
 * `login-status` reads `Signed out` until the first submit, then `Welcome, <username>` when the
 * password is `hunter2` and `Invalid credentials` otherwise. Submitting dismisses the keyboard.
 */
@Composable
private fun LoginScreen() {
    var username by rememberSaveable { mutableStateOf("") }
    var password by rememberSaveable { mutableStateOf("") }
    var status by rememberSaveable { mutableStateOf("Signed out") }
    val focusManager = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        ScreenTitle("Login")
        TextField(
            value = username,
            onValueChange = { username = it },
            modifier = Modifier.fillMaxWidth().testTag("login-username"),
            placeholder = { Text("Username") },
            singleLine = true,
            // Typed text must arrive verbatim, without capitalization or correction.
            keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.None,
                autoCorrectEnabled = false,
                keyboardType = KeyboardType.Text,
                imeAction = ImeAction.Next,
            ),
        )
        TextField(
            value = password,
            onValueChange = { password = it },
            modifier = Modifier.fillMaxWidth().testTag("login-password"),
            placeholder = { Text("Password") },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(
                autoCorrectEnabled = false,
                keyboardType = KeyboardType.Password,
                imeAction = ImeAction.Done,
            ),
        )
        Button(
            onClick = {
                status =
                    if (password == VALID_PASSWORD) "Welcome, $username" else "Invalid credentials"
                focusManager.clearFocus()
                keyboard?.hide()
            },
            modifier = Modifier.testTag("login-submit"),
        ) {
            Text("Sign in")
        }
        Text(text = status, modifier = Modifier.testTag("login-status"))
    }
}

private const val ROW_COUNT = 200

/**
 * 200 rows in a LazyColumn, so only the rows on screen exist in the tree. Tapping `row-<n>` opens
 * `detail-<n>`; `detail-back` or the system back gesture returns to the list at the same scroll
 * position.
 */
@Composable
private fun ListScreen() {
    var openRow by rememberSaveable { mutableStateOf<Int?>(null) }
    val listState = rememberLazyListState()
    val row = openRow
    if (row == null) {
        LazyColumn(state = listState, modifier = Modifier.fillMaxSize()) {
            items(count = ROW_COUNT, key = { index -> index + 1 }) { index ->
                val n = index + 1
                ListRow(n = n, onClick = { openRow = n })
            }
        }
    } else {
        BackHandler { openRow = null }
        RowDetail(n = row, onBack = { openRow = null })
    }
}

/** The row is the tagged Text itself, so one leaf node carries the tag, the text and the action. */
@Composable
private fun ListRow(n: Int, onClick: () -> Unit) {
    Column {
        Text(
            text = "Row $n",
            modifier = Modifier
                .fillMaxWidth()
                .height(48.dp)
                .testTag("row-$n")
                .clickable(role = Role.Button, onClick = onClick)
                .padding(horizontal = 16.dp)
                .wrapContentHeight(Alignment.CenterVertically),
        )
        HorizontalDivider()
    }
}

@Composable
private fun RowDetail(n: Int, onBack: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Button(onClick = onBack, modifier = Modifier.testTag("detail-back")) {
            Text("Back")
        }
        ScreenTitle(text = "Detail $n", modifier = Modifier.testTag("detail-$n"))
    }
}

private const val CELL_COUNT = 500
private const val GRID_COLUMNS = 20
private const val GRID_ROWS = CELL_COUNT / GRID_COLUMNS

/**
 * Exactly 500 clickable cells, composed eagerly in a non-lazy Column. Compose reports only
 * on-screen nodes to accessibility services, so the 20 x 25 grid is sized to the viewport: every
 * cell is on screen, and therefore in the UI automation tree, at once. The `big-count` header is
 * not one of the 500.
 */
@Composable
private fun BigScreen() {
    Column(Modifier.fillMaxSize()) {
        Text(
            text = "$CELL_COUNT elements",
            modifier = Modifier
                .testTag("big-count")
                .semantics { heading() }
                .padding(horizontal = 16.dp, vertical = 12.dp),
            style = MaterialTheme.typography.titleMedium,
        )
        BoxWithConstraints(Modifier.fillMaxSize().padding(horizontal = 8.dp)) {
            val density = LocalDensity.current
            // Whole pixels, so the rows never exceed the viewport by a rounding remainder.
            val cellWidth = with(density) { (constraints.maxWidth / GRID_COLUMNS).toDp() }
            val cellHeight = with(density) { (constraints.maxHeight / GRID_ROWS).toDp() }
            // Cells are smaller than the 48 dp minimum touch target. Compose would expand each
            // cell's touch area to 48 dp and report the overlapping remainders as accessibility
            // bounds; a zero minimum keeps every cell's reported bounds equal to its drawn bounds.
            val viewConfiguration = LocalViewConfiguration.current
            val exactTouchBounds = remember(viewConfiguration) {
                object : ViewConfiguration by viewConfiguration {
                    override val minimumTouchTargetSize: DpSize = DpSize.Zero
                }
            }
            CompositionLocalProvider(LocalViewConfiguration provides exactTouchBounds) {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    for (row in 0 until GRID_ROWS) {
                        Row {
                            for (column in 1..GRID_COLUMNS) {
                                GridCell(
                                    n = row * GRID_COLUMNS + column,
                                    modifier = Modifier.size(cellWidth, cellHeight),
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun GridCell(n: Int, modifier: Modifier) {
    Box(
        modifier = modifier
            .testTag("cell-$n")
            .clickable(role = Role.Button, onClick = {})
            // Placed after clickable, which keeps the click action and role, and drops the text
            // node of the number drawn inside: each cell is a single leaf node.
            .clearAndSetSemantics { contentDescription = "Cell $n" }
            .padding(1.dp)
            .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(3.dp)),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = "$n",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontSize = 8.sp,
            maxLines = 1,
        )
    }
}

/**
 * The spinner turns once per second, updated every display frame, while `anim-toggle` is on. The
 * transition exists only while it runs, so switching it off stops frame production and resets the
 * spinner to 0 degrees.
 */
@Composable
private fun AnimScreen() {
    var running by rememberSaveable { mutableStateOf(true) }
    val rotation: State<Float>? = if (running) rememberSpinnerRotation() else null
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(32.dp),
    ) {
        ScreenTitle("Animation")
        Box(
            modifier = Modifier
                .size(120.dp)
                .testTag("anim-spinner")
                .semantics { contentDescription = "Spinner" }
                // Read in the layer block, so each frame redraws the layer without recomposing.
                .graphicsLayer { rotationZ = rotation?.value ?: 0f }
                .background(MaterialTheme.colorScheme.primary, RoundedCornerShape(24.dp)),
            contentAlignment = Alignment.TopCenter,
        ) {
            Box(
                Modifier
                    .padding(top = 12.dp)
                    .size(width = 12.dp, height = 36.dp)
                    .background(MaterialTheme.colorScheme.onPrimary, RoundedCornerShape(6.dp)),
            )
        }
        // The whole row is the switch: one node carries the tag, the label and the checked state.
        Row(
            modifier = Modifier
                .testTag("anim-toggle")
                .toggleable(value = running, role = Role.Switch, onValueChange = { running = it })
                // Placed after toggleable, which keeps the toggle action, state and role, and drops
                // the nodes of the caption and of the switch graphic.
                .clearAndSetSemantics { contentDescription = "Animate" }
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Animate")
            Switch(checked = running, onCheckedChange = null)
        }
    }
}

@Composable
private fun rememberSpinnerRotation(): State<Float> =
    rememberInfiniteTransition(label = "spinner").animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(tween(durationMillis = 1000, easing = LinearEasing)),
        label = "rotation",
    )

/**
 * `system-permission` opens the platform's location permission dialog, a system-owned window.
 * `system-permission-status` reads `Permission: unknown` until the request returns, then
 * `Permission: granted` when precise or approximate location was granted and `Permission: denied`
 * otherwise. `system-alert` opens an in-app dialog.
 */
@Composable
private fun SystemScreen() {
    val context = LocalContext.current
    var permission by rememberSaveable { mutableStateOf("Permission: unknown") }
    var showAlert by rememberSaveable { mutableStateOf(false) }
    val permissionRequest =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { fine ->
            val coarse = context.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) ==
                PackageManager.PERMISSION_GRANTED
            permission = if (fine || coarse) "Permission: granted" else "Permission: denied"
        }
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        ScreenTitle("System")
        Button(
            onClick = { permissionRequest.launch(Manifest.permission.ACCESS_FINE_LOCATION) },
            modifier = Modifier.testTag("system-permission"),
        ) {
            Text("Request location permission")
        }
        Text(text = permission, modifier = Modifier.testTag("system-permission-status"))
        Button(onClick = { showAlert = true }, modifier = Modifier.testTag("system-alert")) {
            Text("Show in-app alert")
        }
    }
    if (showAlert) {
        AlertDialog(
            onDismissRequest = { showAlert = false },
            confirmButton = {
                TextButton(
                    onClick = { showAlert = false },
                    modifier = Modifier.testTag("inapp-alert-ok"),
                ) {
                    Text("OK")
                }
            },
            // The dialog is a separate window with its own semantics tree, so it sets the flag too.
            modifier = Modifier.semantics { testTagsAsResourceId = true },
            title = { Text("In-app alert") },
            text = { Text("This alert is presented by the app.") },
        )
    }
}
