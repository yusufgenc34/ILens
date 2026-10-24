#pragma warning disable
namespace ILens.Editing
{
    public class Counter : ILens.Editing.ICounter
    {
        public int Value;
        public readonly int ReadOnly;
        public static string Name;
        public extern Counter(int value);
        public virtual int Read() => throw null;
public int Increase(int amount)
{
    int doubled = amount * 2;
    System.Console.WriteLine("adding " + doubled);
    this.Value += doubled;
    return this.Value;
}
        public void Rename(string name) => throw null;
        public int Property { get => throw null; set => throw null; }
    }
}
