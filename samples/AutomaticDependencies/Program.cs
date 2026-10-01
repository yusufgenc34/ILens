using System;
using System.Collections.Generic;
// Compiled solely as a managed executable fixture. Tests never execute it.
internal static class Program {
    private static void Main() {
        Console.WriteLine(new List<string> { "inspection", "fixture" }.Count);
    }
}
