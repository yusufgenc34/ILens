namespace ILens.Samples;
public static class WorkspacePatterns {
    public static string DependencyProperty(ILens.Dependency.Widget widget, string value) {
        widget.Text = value;
        return widget.Text;
    }
    public static string OrdinaryGetterName(ILens.Dependency.Widget widget) => widget.get_NotAProperty();
    public static int ReadField(ILens.Dependency.Widget widget) => widget.Number;
    public static void WriteField(ILens.Dependency.Widget widget, int value) { widget.Number = value; }
    public static string UnicodeBeforeCall(ILens.Dependency.Widget widget) => "😀" + widget.Text;
}
public class DerivedWidget : ILens.Dependency.Widget { }
public class WidgetImplementation : ILens.Dependency.IWidget {
    public string Text { get; set; } = "";
}

public enum ByteMode : byte { None, Ready }
public enum LongMode : long { None, Large = 5000000000 }
public static class EnumEditing {
    public static ByteMode ChooseMode(int value) => value > 0 ? ByteMode.Ready : ByteMode.None;
    public static ByteMode ReadMode(ByteMode[] values) => values[0];
    public static void StoreMode(ByteMode[] values, ByteMode value) { values[0] = value; }
    public static object BoxMode(ByteMode value) => value;
    public static ByteMode UnboxMode(object value) => (ByteMode)value;
    public static LongMode ReadLongMode(LongMode[] values) => values[0];
    public static void StoreLongMode(LongMode[] values, LongMode value) { values[0] = value; }
}
