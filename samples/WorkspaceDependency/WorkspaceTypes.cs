namespace ILens.Dependency;
public interface IWidget { string Text { get; set; } }
public class Widget : IWidget {
    public string Text { get; set; } = "";
    public int Number;
    public string get_NotAProperty() => "ordinary method";
}
