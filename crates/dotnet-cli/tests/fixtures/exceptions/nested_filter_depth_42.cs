using System;
public class Program {
 static void Thrower() { throw new Exception("original"); }
 static bool Filter(int depth) {
  Console.WriteLine(depth);
  if (depth == 0) return true;
  try { Thrower(); }
  catch (Exception) when (Filter(depth - 1)) { Console.WriteLine("caught"); }
  return true;
 }
 public static int Main() {
  try { Thrower(); }
  catch (Exception) when (Filter(2)) { return 42; }
  return 1;
 }
}
