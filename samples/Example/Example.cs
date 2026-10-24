// The example opened from ILens's start page: a few small, readable types.
// Compiled for inspection only; ILens never runs it.
using System;

namespace ILens.Example;

public class Greeter
{
    private readonly string greeting;

    public Greeter(string greeting)
    {
        this.greeting = greeting;
    }

    public string Greet(string name)
    {
        if (string.IsNullOrEmpty(name))
            return greeting + "!";
        return greeting + ", " + name + "!";
    }
}

public static class Calculator
{
    public static int Add(int a, int b) => a + b;

    public static int Factorial(int n)
    {
        int result = 1;
        for (int i = 2; i <= n; i++)
            result *= i;
        return result;
    }

    public static double Average(int[] values)
    {
        if (values.Length == 0)
            throw new ArgumentException("At least one value is required.", nameof(values));
        int sum = 0;
        foreach (int value in values)
            sum += value;
        return (double)sum / values.Length;
    }
}

public enum Shape
{
    Circle,
    Square,
}

public class Person
{
    public string Name { get; set; } = "";

    public int Age { get; set; }

    public bool IsAdult => Age >= 18;

    public override string ToString() => Name + " (" + Age + ")";
}

public static class Geometry
{
    public static double Area(Shape shape, double size)
    {
        switch (shape)
        {
            case Shape.Circle:
                return Math.PI * size * size;
            case Shape.Square:
                return size * size;
            default:
                return 0;
        }
    }
}
