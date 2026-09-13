namespace ILens.Readability;

public sealed class FormValues
{
    public string Text { get; set; } = "fixture";
    public bool Checked { get; set; }
    // This is an ordinary method, deliberately not a property accessor.
    public string get_NotAProperty() => "method";

    public string BuildMessage()
    {
        try
        {
            string separator = "|";
            string[] values = new string[12];
            values[0] = separator;
            values[1] = Text;
            values[2] = separator;
            values[3] = Text;
            values[4] = separator;
            values[5] = Convert.ToString(Checked);
            values[6] = separator;
            values[7] = Text;
            values[8] = separator;
            values[9] = Convert.ToString(Checked);
            values[10] = separator;
            values[11] = get_NotAProperty();
            return string.Concat(values);
        }
        catch (Exception error)
        {
            return error.ToString();
        }
    }

    public static string ExceptionMessage(Exception error) => error.Message;
    public string PropertyRead() => Text;
    public string OrdinaryMethod() => get_NotAProperty();
    public static int[] Initializer(int value) => new int[] { value, value + 1, 42 };
    public static int[] PartialArray(int value)
    {
        int[] values = new int[3];
        try { values[0] = value; values[1] = 10 / value; values[2] = 42; }
        catch { return values; }
        return values;
    }
}
