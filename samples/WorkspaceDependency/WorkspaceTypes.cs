namespace ILens.Dependency;
public interface IWidget { string Text { get; set; } }
public class Widget : IWidget {
    public string Text { get; set; } = "";
    public int Number;
    public string get_NotAProperty() => "ordinary method";
}
public enum Level { Low = 1, High = 2 }
public struct Size { public int Width; public int Height; }
public class Gadget
{
    public int Open;
    internal int Secret;
    protected int Guarded;
}
