

# Plan

This solution will be done using the following plan:
1 - Minimial e2e working example;
2 - e2e snapshot framework;
3 - Using the test framework, implement all features;
4 - Improve the architecture/code;
5 - Improve the performance;

# AI

I will use AI in the following steps:

1 - Transform the spec to a AI digestable markdown;
2 - Use the MD to check all features are implemented;
3 - Search for bugs and problems;
4 - Help with the test framework and writing and document tests;

# Devlog

I will try to create here the most realistic devlog of the steps I took whilst developing this solution.

# First step

I will expedite a minimal example working. That means that code does not need to be good. I just want something working e2e. In this case will be a simple CLI parsing a one-line CSV and printing the output.

I will use precise versioning in `Cargo.toml` as a security measure agains supply chain attacks.

# Test framework

Now that we can process the simple CSV file, I want to create the test framework. Although I do see the value of "unit tests", in this case, I think e2e snapshot tests will be a better fit. Why? Because they will be visually simpler to understand what they are testing, and their final result.

For that I will use `insta` and `libtest-mimic`. That means that a simple `cargo t` will run all tests as normal.

A "polemic" decision is why use `cargo r` to run the app, and not call a function inside the crate directly. There are disadvantages of callign the compiled exe for sure, but there are also advantages: we can run test in parallel more easily; one crashing test does not kill others; we can capture log and put in the snapshot etc...

It is a little bit ugly, I will give you that, but is something I already use in other repos, and is something I like.
