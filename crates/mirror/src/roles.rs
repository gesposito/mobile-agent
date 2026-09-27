//! Roles from platform classes and node states.
//!
//! Android nodes carry their widget class (Compose reports the class of the
//! role it was given); the Simulator driver passes the `XCUIElementType`
//! name, or the UIKit class for elements typed `Other`, so that an alert
//! controller can be recognized. States decide what a class does not:
//! an editable node is a text box, a checkable one a checkbox, a clickable
//! one a button.

use protocol::snapshot::NodeFlags;

use crate::snapshot::Role;

const XCUI_PREFIX: &str = "XCUIElementType";

/// The role a class names on its own, if any.
pub(crate) fn class_role(class: &str) -> Option<Role> {
    if let Some(short) = class.strip_prefix(XCUI_PREFIX) {
        return xcui_role(short);
    }
    let simple = class.rsplit(['.', '$']).next().unwrap_or(class);
    simple_role(simple)
}

fn xcui_role(name: &str) -> Option<Role> {
    Some(match name {
        "Button" | "Key" => Role::Button,
        "Link" => Role::Link,
        "TextField" | "SecureTextField" | "TextView" => Role::Textbox,
        "SearchField" => Role::Searchbox,
        "Switch" | "Toggle" => Role::Switch,
        "CheckBox" => Role::Checkbox,
        "RadioButton" => Role::Radio,
        "Slider" | "Stepper" => Role::Slider,
        "Picker" | "PickerWheel" | "PopUpButton" | "ComboBox" | "DatePicker" => Role::Combobox,
        "Tab" => Role::Tab,
        "TabBar" | "TabGroup" | "SegmentedControl" => Role::Tablist,
        "MenuItem" | "MenuBarItem" => Role::Menuitem,
        "StaticText" => Role::Text,
        "Image" | "Icon" => Role::Image,
        "Cell" => Role::Cell,
        "Heading" => Role::Heading,
        "Table" | "CollectionView" | "Outline" | "Browser" => Role::List,
        "Grid" => Role::Grid,
        "Alert" => Role::Alert,
        "Sheet" | "Dialog" | "Popover" => Role::Dialog,
        "ActivityIndicator" | "ProgressIndicator" | "LevelIndicator" => Role::Progressbar,
        "NavigationBar" | "Toolbar" => Role::Toolbar,
        "Keyboard" => Role::Keyboard,
        "WebView" => Role::Webview,
        "ScrollView" | "Other" | "Group" | "Window" | "Application" | "LayoutArea"
        | "LayoutItem" | "Any" => Role::Generic,
        _ => return None,
    })
}

fn simple_role(name: &str) -> Option<Role> {
    Some(match name {
        "Button"
        | "ImageButton"
        | "MaterialButton"
        | "AppCompatButton"
        | "AppCompatImageButton"
        | "FloatingActionButton"
        | "ExtendedFloatingActionButton"
        | "UIButton"
        | "_UIButtonBarButton" => Role::Button,
        "EditText"
        | "AppCompatEditText"
        | "TextInputEditText"
        | "AutoCompleteTextView"
        | "AppCompatAutoCompleteTextView"
        | "MultiAutoCompleteTextView"
        | "UITextField"
        | "UISearchBarTextField"
        | "RCTUITextField"
        | "RCTUITextView" => Role::Textbox,
        "CheckBox"
        | "AppCompatCheckBox"
        | "MaterialCheckBox"
        | "CheckedTextView"
        | "AppCompatCheckedTextView" => Role::Checkbox,
        "Switch" | "SwitchCompat" | "SwitchMaterial" | "MaterialSwitch" | "ToggleButton"
        | "UISwitch" => Role::Switch,
        "RadioButton" | "AppCompatRadioButton" | "MaterialRadioButton" => Role::Radio,
        "SeekBar" | "AppCompatSeekBar" | "RatingBar" | "AppCompatRatingBar" | "Slider"
        | "RangeSlider" | "UISlider" => Role::Slider,
        "Spinner" | "AppCompatSpinner" => Role::Combobox,
        "TabView" | "_UITabButton" | "UITabBarButton" => Role::Tab,
        "TabWidget" | "TabLayout" | "UITabBar" => Role::Tablist,
        "TextView"
        | "AppCompatTextView"
        | "MaterialTextView"
        | "UILabel"
        | "RCTParagraphComponentView"
        | "RCTTextView" => Role::Text,
        "ImageView"
        | "AppCompatImageView"
        | "ShapeableImageView"
        | "UIImageView"
        | "RCTImageComponentView" => Role::Image,
        "ProgressBar"
        | "ContentLoadingProgressBar"
        | "CircularProgressIndicator"
        | "LinearProgressIndicator"
        | "UIActivityIndicatorView"
        | "UIProgressView" => Role::Progressbar,
        "RecyclerView" | "ListView" | "ExpandableListView" | "UITableView" | "UICollectionView" => {
            Role::List
        }
        "GridView" => Role::Grid,
        "Toolbar" | "MaterialToolbar" | "ActionBarContainer" | "UINavigationBar" | "UIToolbar" => {
            Role::Toolbar
        }
        "WebView" | "WKWebView" => Role::Webview,
        "UITableViewCell" | "UICollectionViewCell" => Role::Cell,
        "_UIAlertControllerPhoneTVMacView" | "_UIAlertControllerView" | "UIAlertControllerView" => {
            Role::Alert
        }
        "Heading" => Role::Heading,
        "View"
        | "ViewGroup"
        | "FrameLayout"
        | "LinearLayout"
        | "RelativeLayout"
        | "ConstraintLayout"
        | "CoordinatorLayout"
        | "ScrollView"
        | "NestedScrollView"
        | "HorizontalScrollView"
        | "ViewPager"
        | "ViewPager2"
        | "ComposeView"
        | "AndroidComposeView"
        | "DecorView"
        | "UIView"
        | "UIScrollView"
        | "RCTViewComponentView"
        | "RCTScrollViewComponentView" => Role::Generic,
        _ => return None,
    })
}

/// Whether the node takes input itself: a tap, a long press or text.
pub(crate) fn acts(flags: NodeFlags) -> bool {
    flags.contains(NodeFlags::CLICKABLE)
        || flags.contains(NodeFlags::LONG_CLICKABLE)
        || flags.contains(NodeFlags::EDITABLE)
}

/// The role of a node from its class and states.
pub(crate) fn role(class: &str, flags: NodeFlags, has_text: bool) -> Role {
    let base = class_role(class);
    match base {
        Some(Role::Generic | Role::Group | Role::Text | Role::Image) | None => {
            if flags.contains(NodeFlags::EDITABLE) {
                Role::Textbox
            } else if flags.contains(NodeFlags::CHECKABLE) {
                Role::Checkbox
            } else if acts(flags) {
                Role::Button
            } else if let Some(role) = base {
                role
            } else if has_text {
                Role::Text
            } else {
                Role::Generic
            }
        }
        Some(role) => role,
    }
}
