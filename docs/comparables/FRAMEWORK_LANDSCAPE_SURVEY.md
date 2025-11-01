# Spiking Neural Network Framework Landscape Survey

**Document Type**: Landscape Survey & Multi-Framework Comparison  
**Date**: November 1, 2025  
**Version**: 1.0  
**Status**: Active

---

## Executive Summary

This document provides a high-level comparative survey of **ten major spiking neural network frameworks and platforms**: **FEAGI**, **Intel Lava**, **Nengo**, **CARLsim**, **snnTorch**, **GeNN**, **NEST**, **Brian2**, **SpiNNaker**, and **EONS**. Each framework serves distinct use cases with unique strengths, and this survey helps researchers, developers, and decision-makers quickly identify which framework(s) best suit their needs.

**Key Finding**: These frameworks are **complementary**, not competitive. Most real-world projects could benefit from combining multiple frameworks at different stages (e.g., training with snnTorch, deploying with FEAGI, validating with GeNN).

**Notable**: Only **FEAGI** and **EONS** use evolutionary algorithms to optimize brain/network structure - all others use manual design, supervised learning, or mathematical construction.

---

## Framework Overview Table

| Framework | Organization | Language | Primary Focus | License | First Release |
|-----------|--------------|----------|---------------|---------|---------------|
| **FEAGI** | Neuraville Inc. | Rust + Python | Evolutionary AGI, embodied agents | Apache 2.0 | 2016 (2.0 in 2024) |
| **EONS** | ORNL (DOE) | Python | Evolutionary neuromorphic optimizer | DOE/Open | 2020 |
| **Lava** | Intel Corporation | Python + C/C++ | Neuromorphic computing, Loihi | BSD-3 / LGPL | 2021 |
| **Nengo** | Applied Brain Research | Python | Cognitive modeling, NEF | BSD-3 | 2013 |
| **CARLsim** | UC Irvine CARL Lab | C++ + CUDA | Visual cortex, GPU simulation | MIT-like | 2009 |
| **snnTorch** | Open source | Python (PyTorch) | Deep learning with SNNs | MIT | 2021 |
| **GeNN** | University of Sussex | C++ + CUDA | Large-scale GPU simulation | GPL/LGPL | 2016 |
| **NEST** | NEST Initiative | C++ + Python | Brain simulation standard | GPL-2.0 | 1993 |
| **Brian2** | Imperial College | Python | Easy equation-based simulation | CeCILL | 2008 |
| **SpiNNaker** | Univ of Manchester | Hardware + Python | Neuromorphic hardware platform | GPL | 2006 |

---

## Architectural Paradigms

| Framework | Design Paradigm | Brain Construction | Theoretical Foundation |
|-----------|-----------------|-------------------|------------------------|
| **FEAGI** | Genome → Embryogenesis | Automatic (biological development) | Evolutionary developmental neuroscience |
| **EONS** | Evolutionary (GA) | Automatic (evolution) | Genetic algorithms + multi-objective |
| **Lava** | Process composition | Programmatic (CSP processes) | Communicating Sequential Processes |
| **Nengo** | Function → NEF | Programmatic (mathematical) | Neural Engineering Framework |
| **CARLsim** | Network definition | Programmatic (C++ API) | GPU-accelerated simulation |
| **snnTorch** | Layer composition | Programmatic (PyTorch) | Deep learning + SNNs |
| **GeNN** | Code generation | Programmatic (C++/Python) | Optimized CUDA kernels |
| **NEST** | Network definition | Programmatic (Python) | Biologically accurate simulation |
| **Brian2** | Equation-based | Programmatic (equations) | Flexible equation specification |
| **SpiNNaker** | Hardware platform | Programmatic (PyNN/sPyNNaker) | Massively parallel neuromorphic |

---

## Hardware & Platform Support

| Framework | CPU | GPU | Neuromorphic HW | Embedded | Cloud/K8s |
|-----------|-----|-----|-----------------|----------|-----------|
| **FEAGI** | ✅ Rust (high-perf) | 📋 Planned | 📋 Planned | 🚧 RTOS | ✅ Docker, K8s |
| **Lava** | ✅ Python/C | ✅ CUDA | ✅ Loihi (1000x power) | ❌ Limited | ⚠️ Manual |
| **Nengo** | ✅ NumPy | ✅ TensorFlow/PyTorch | ✅ Loihi, SpiNNaker, FPGA | ⚠️ Limited | ⚠️ Manual |
| **CARLsim** | ⚠️ Slow | ✅ CUDA (10-50x) | ❌ No | ❌ No | ⚠️ Manual |
| **snnTorch** | ✅ PyTorch | ✅ CUDA, ROCm, TPU | 🚧 Research | ⚠️ Limited | ✅ Colab, Kaggle |
| **GeNN** | ⚠️ Slow | ✅ CUDA (10-100x) | ❌ No | ❌ No | ⚠️ Manual |

**Key**:
- ✅ = Full support
- 🚧 = In progress
- 📋 = Planned
- ⚠️ = Limited/Manual
- ❌ = Not supported

---

## Learning Mechanisms Comparison

| Framework | Primary Learning | Supervised | Unsupervised | Evolutionary | Online Learning |
|-----------|------------------|------------|--------------|--------------|-----------------|
| **FEAGI** | STDP + Evolutionary | ❌ | ✅ STDP | ✅ Genome evolution | ✅ Continual |
| **EONS** | Evolutionary (GA) | ⚠️ Task-based | ⚠️ Task-based | ✅ Network evolution | ❌ Static after evolution |
| **Lava** | SLAYER (backprop) | ✅ SLAYER | ✅ STDP | ❌ | ✅ STDP |
| **Nengo** | NEF + Backprop | ✅ NengoDL | ✅ PES, BCM | ❌ | ✅ PES |
| **CARLsim** | STDP + DA-STDP | ⚠️ Limited | ✅ STDP, STP | ⚠️ ECJ (params only) | ✅ STDP |
| **snnTorch** | Backpropagation | ✅ BPTT | ⚠️ Limited | ❌ | ⚠️ Not primary |
| **GeNN** | STDP variants | ⚠️ Limited | ✅ STDP, STP | ❌ | ✅ STDP |
| **NEST** | STDP + STP | ⚠️ Limited | ✅ STDP, STP, structural | ❌ | ✅ STDP |
| **Brian2** | User-defined | ✅ Possible | ✅ Any (equations) | ❌ | ✅ User-defined |
| **SpiNNaker** | STDP (via software) | ⚠️ Limited | ✅ STDP | ❌ | ✅ STDP |

**CRITICAL**: Only **FEAGI** and **EONS** use evolutionary algorithms (2 out of 10)

**Data Requirements**:
- **FEAGI**: None (unsupervised evolutionary)
- **EONS**: Task-dependent (fitness evaluation)
- **Lava**: Labeled data (SLAYER) or none (STDP)
- **Nengo**: Labeled data (NengoDL) or none (NEF/PES)
- **CARLsim**: None (unsupervised plasticity)
- **snnTorch**: Labeled data (supervised)
- **GeNN**: None (unsupervised plasticity)
- **NEST**: None (unsupervised plasticity)
- **Brian2**: User-defined
- **SpiNNaker**: Depends on software used

---

## Scale & Performance

| Framework | Max Neurons | Max Synapses | CPU Performance | GPU Speedup | Real-Time Capable |
|-----------|-------------|--------------|-----------------|-------------|-------------------|
| **FEAGI** | Millions | Tens of millions | High (Rust) | 📋 Planned | ✅ <10ms latency |
| **EONS** | Thousands-Millions | Task-optimized | Via neuromorphic | ✅ On Loihi | ✅ μs on Loihi |
| **Lava** | Millions | Millions | Moderate | ✅ Yes | ✅ μs on Loihi |
| **Nengo** | Millions (10M+) | Millions | Moderate (NumPy) | ✅ Via NengoDL | ⚠️ Backend-dependent |
| **CARLsim** | 10M+ | Billions | Low | ✅ 10-50x | ✅ On GPU |
| **snnTorch** | Millions | Millions | Moderate | ✅ PyTorch | ⚠️ Inference-focused |
| **GeNN** | **100M+** | **Billions** | Low | ✅ **10-100x** | ✅ On GPU |
| **NEST** | **Millions-Billions** | **Billions** | High (C++) | ⚠️ Experimental | ⚠️ Simulation |
| **Brian2** | 100K-1M | Millions | Moderate | ✅ Brian2GeNN | ⚠️ Simulation |
| **SpiNNaker** | **Up to 1 billion** | **Trillions** | N/A (hardware) | N/A (hardware) | ✅ 1000x real-time |

**Performance Leaders**:
- **Scale**: SpiNNaker (1 billion neurons on hardware), NEST (billions on HPC), GeNN (100M+ on GPU)
- **CPU Performance**: FEAGI (Rust core), NEST (C++ core)
- **GPU Speedup**: GeNN (10-100x), CARLsim (10-50x)
- **Power Efficiency**: Lava (1000x on Loihi), EONS (<1W on Loihi), SpiNNaker (100W for 1M neurons)
- **Real-Time Control**: FEAGI (<10ms sensory-motor)
- **Ease of Use**: Brian2 (equation-based), snnTorch (PyTorch)

---

## Key Differentiators by Framework

### Evolutionary Frameworks (Only 2 out of 10!)

**FEAGI and EONS are unique** - they're the **only frameworks** that use evolutionary algorithms to optimize brain/network structure. All others require manual design, supervised learning, or mathematical construction.

---

### FEAGI - Evolutionary Embodied AI
**Unique Features**:
- 🧬 **Neuroembryogenesis**: Only framework with genome-to-brain development
- 🤖 **Multi-Agent Native**: Built-in ZMQ coordination
- 🔄 **Evolutionary Optimization**: Genome-level evolution
- 🏭 **Production Ready**: Docker, Kubernetes, PyPI

**Best For**:
- Autonomous robots with real-time control
- Multi-agent coordination (swarms)
- AGI research with evolutionary optimization
- CPU-only deployment (no GPU required)

**Notable**: Only framework with genome-to-brain development (neuroembryogenesis)

---

### EONS - Evolutionary Hardware Optimizer
**Unique Features**:
- 🧬 **Evolutionary Algorithm**: Genetic algorithm (GA) optimization (1 of only 2!)
- 🎯 **Multi-Objective**: Pareto optimization (accuracy, size, energy)
- ⚡ **Hardware-Aware**: Optimized for neuromorphic chips (Loihi)
- 🏛️ **ORNL Backing**: U.S. Department of Energy research

**Best For**:
- Neuromorphic hardware deployment (Loihi)
- Ultra-low power edge computing (<1W)
- Automated network design (no manual configuration)
- Multi-objective optimization

**Notable**: Only other evolutionary framework (with FEAGI); ORNL research project

---

### Lava - Intel Neuromorphic Platform
**Unique Features**:
- ⚡ **Loihi Integration**: Native Intel Loihi support
- 🔋 **Power Efficiency**: 1000x better on neuromorphic hardware
- 🏗️ **Process-Based**: CSP paradigm, multi-backend
- 📚 **Rich Ecosystem**: SLAYER, DNF, optimization libraries

**Best For**:
- Intel Loihi deployment
- Ultra-low power edge inference
- Neuromorphic computing research
- Constrained optimization problems

**Notable**: Only framework with production Loihi support

---

### Nengo - Cognitive Modeling Framework
**Unique Features**:
- 🔢 **Neural Engineering Framework**: Mathematical rigor
- 🧠 **Spaun**: World's largest functional brain (2.5M neurons, 8 tasks)
- 🎓 **Educational Excellence**: Summer school, extensive tutorials
- 🔌 **Multiple Backends**: Loihi, SpiNNaker, FPGA, GPU

**Best For**:
- Cognitive phenomenon modeling
- Computational neuroscience education
- NEF and SPA research
- Neuromorphic hardware deployment

**Notable**: Published in *Science* (Spaun model)

---

### CARLsim - Visual Cortex GPU Simulator
**Unique Features**:
- 🎮 **GPU Acceleration**: 10-50x speedup (CUDA)
- 👁️ **Visual Cortex**: Specialized V1/V2/MT models
- 🔧 **Parameter Tuning**: Automated ECJ framework
- 🧪 **Rich Plasticity**: STDP, DA-STDP, STP, homeostasis

**Best For**:
- Visual cortex research
- GPU-accelerated neuroscience
- Automated parameter optimization
- Dopamine modulation studies

**Notable**: 100+ citations, UC Irvine CARL Lab

---

### snnTorch - PyTorch Deep Learning SNNs
**Unique Features**:
- 🔥 **PyTorch Integration**: Seamless DL workflow
- 📖 **Tutorial Excellence**: 20+ comprehensive notebooks
- ⚡ **Fast Training**: GPU-accelerated backpropagation
- 🎓 **Educational Focus**: Easy to learn

**Best For**:
- Supervised SNN training
- PyTorch ecosystem integration
- Neuromorphic AI education
- Energy-efficient classification

**Notable**: Most accessible for deep learning practitioners

---

### GeNN - Large-Scale Code Generation
**Unique Features**:
- 🏗️ **Code Generation**: Optimized CUDA kernel generation
- 📏 **Massive Scale**: 100M+ neurons, billion synapses
- 💾 **Procedural Connectivity**: 10-1000x memory reduction
- 📊 **Proven Performance**: Outperforms neuromorphic HW (published)

**Best For**:
- Large-scale brain simulation
- Computational neuroscience research
- GPU performance benchmarking
- Memory-efficient networks

**Notable**: Published in *Nature Computational Science* (2021)

---

## Use Case Decision Matrix

| Use Case | FEAGI | Lava | Nengo | CARLsim | snnTorch | GeNN |
|----------|-------|------|-------|---------|----------|------|
| **Autonomous Robots** | ✅✅✅ | ⚠️ | ⚠️ | ⚠️ | ❌ | ❌ |
| **Multi-Agent Coordination** | ✅✅✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Real-Time Control** | ✅✅✅ | ✅ | ⚠️ | ⚠️ | ⚠️ | ⚠️ |
| **Evolutionary Optimization** | ✅✅✅ | ❌ | ❌ | ⚠️ | ❌ | ❌ |
| **Cognitive Modeling** | ⚠️ | ⚠️ | ✅✅✅ | ⚠️ | ❌ | ❌ |
| **Education/Teaching** | ⚠️ | ⚠️ | ✅✅✅ | ⚠️ | ✅✅✅ | ⚠️ |
| **Supervised Classification** | ❌ | ✅✅ | ✅✅ | ⚠️ | ✅✅✅ | ⚠️ |
| **Neuromorphic Deployment** | 📋 | ✅✅✅ | ✅✅ | ❌ | 🚧 | ❌ |
| **GPU Acceleration** | 📋 | ✅ | ✅ | ✅✅✅ | ✅✅ | ✅✅✅ |
| **Large-Scale (100M+)** | ⚠️ | ⚠️ | ✅ | ✅ | ⚠️ | ✅✅✅ |
| **Visual Cortex Research** | ⚠️ | ⚠️ | ⚠️ | ✅✅✅ | ⚠️ | ✅ |
| **Production Deployment** | ✅✅✅ | ⚠️ | ⚠️ | ❌ | ⚠️ | ❌ |
| **Embedded Systems** | 🚧 | ❌ | ❌ | ❌ | ❌ | ❌ |

**Rating**:
- ✅✅✅ = Excellent, primary use case
- ✅✅ = Very good, well-supported
- ✅ = Good, possible
- ⚠️ = Limited support, requires work
- 🚧 = In development
- 📋 = Planned
- ❌ = Not supported/not designed for

---

## Technical Capabilities Comparison

### Programming & Interfaces

| Framework | Primary Language | API Languages | GUI | Learning Curve |
|-----------|------------------|---------------|-----|----------------|
| **FEAGI** | Rust + Python | Python, REST API | Brain Visualizer (3D) | Steep (neuroscience) |
| **Lava** | Python + C/C++ | Python | ❌ External tools | Moderate (CSP paradigm) |
| **Nengo** | Python | Python | ✅ NengoGUI (interactive) | Gentle (Python users) |
| **CARLsim** | C++ + CUDA | C++, limited Python | ❌ External tools | Moderate (C++ required) |
| **snnTorch** | Python (PyTorch) | Python | ❌ Jupyter | Easy (PyTorch users) |
| **GeNN** | C++ + CUDA | C++, PyGeNN | ❌ External tools | Moderate (C++/Python) |

### Neuron Model Support

| Framework | Default Model | Available Models | Custom Models | Biological Detail |
|-----------|---------------|------------------|---------------|-------------------|
| **FEAGI** | LIF | LIF, Memory (+ more planned) | Genome-based | Moderate |
| **Lava** | LIF | LIF, custom | Python classes | Moderate |
| **Nengo** | LIF | LIF, Izhikevich, custom | Python classes | Moderate |
| **CARLsim** | Izhikevich | Izhikevich, LIF, adaptive | C++ code | High |
| **snnTorch** | Leaky | Leaky, Synaptic, Alpha, RLeaky (8+) | Python classes | Moderate |
| **GeNN** | LIF | LIF, Izhikevich, HH, Poisson | C++ snippets | High |

### Plasticity Mechanisms

| Framework | STDP | Dopamine | Short-Term (STP) | Homeostatic | Custom Rules |
|-----------|------|----------|------------------|-------------|--------------|
| **FEAGI** | ✅ Basic | 📋 Planned | ⚠️ Limited | ⚠️ Limited | Genome-based |
| **Lava** | ✅ Multi | ✅ R-STDP | ⚠️ Limited | ❌ | ✅ Python |
| **Nengo** | ✅ Yes | ⚠️ Via custom | ⚠️ Via custom | ⚠️ Via custom | ✅ Python |
| **CARLsim** | ✅ Multi | ✅ DA-STDP | ✅ Yes | ✅ Yes | ✅ C++ |
| **snnTorch** | ⚠️ Limited | ❌ | ❌ | ❌ | ✅ Python |
| **GeNN** | ✅ Multi | ✅ Yes | ✅ Yes | ⚠️ Limited | ✅ C++ |

**Plasticity Leaders**:
- **Most Complete**: CARLsim (STDP, DA-STDP, STP, homeostasis)
- **Research Flexibility**: GeNN (custom C++ snippets, GPU-accelerated)
- **Evolutionary**: FEAGI (unique genome-level evolution)

---

## Agent Integration & Real-Time Control

| Framework | Multi-Agent Support | Real-Time Latency | Communication Protocol | Agent SDK |
|-----------|---------------------|-------------------|------------------------|-----------|
| **FEAGI** | ✅✅✅ Native (ZMQ) | <10ms sensory-motor | ZMQ multi-stream | ✅ Python, Rust (planned) |
| **Lava** | ❌ Manual | μs (Loihi), ms (CPU/GPU) | In-process channels | ❌ Manual |
| **Nengo** | ❌ Manual | ms-μs (backend-dependent) | In-process | ❌ Manual |
| **CARLsim** | ❌ Manual | ms (GPU), slow (CPU) | Manual C++ | ❌ Manual |
| **snnTorch** | ❌ Manual | ms (GPU inference) | Manual | ❌ Manual |
| **GeNN** | ❌ Manual | <1ms (GPU) | Manual C++/Python | ❌ Manual |

**Real-Time Control Leader**: FEAGI (only framework with native multi-agent real-time control)

---

## Training & Optimization

| Framework | Training Speed | Training Method | Data Required | GPU Training |
|-----------|----------------|-----------------|---------------|--------------|
| **FEAGI** | Slow (evolutionary) | Evolutionary + STDP | None | 📋 Planned |
| **Lava** | Fast (SLAYER) | Backpropagation | Labeled | ✅ Yes |
| **Nengo** | Instant (NEF) or Fast (DL) | NEF or backprop | NEF: none, DL: labeled | ✅ NengoDL |
| **CARLsim** | Online (STDP) | STDP, DA-STDP | None | ✅ Yes |
| **snnTorch** | Fast (backprop) | Backpropagation | Labeled | ✅ Yes |
| **GeNN** | Online (STDP) | STDP, STP | None | ✅ Yes |

**Training Speed Leaders**:
- **Fastest**: snnTorch, Lava (supervised backprop)
- **No Data Required**: FEAGI, CARLsim, GeNN (unsupervised)
- **Instant**: Nengo (NEF computes weights mathematically)

---

## Deployment & Production Readiness

| Framework | Package Management | Containerization | Cloud Support | Binary Size | Installation Ease |
|-----------|-------------------|------------------|---------------|-------------|-------------------|
| **FEAGI** | ✅ PyPI | ✅ Docker, K8s | ✅ Kubernetes | ~50MB | Easy (pip) |
| **Lava** | ✅ PyPI | ⚠️ Manual | ⚠️ Manual | ~100MB | Easy (pip) |
| **Nengo** | ✅ PyPI | ⚠️ Manual | ⚠️ Manual | ~100MB | Easy (pip) |
| **CARLsim** | ❌ Build from source | ⚠️ Manual | ⚠️ Manual | Compiled | Hard (CMake + CUDA) |
| **snnTorch** | ✅ PyPI | ⚠️ Manual | ✅ Colab, Kaggle | ~50MB | Easy (pip) |
| **GeNN** | ✅ PyPI, Conda | ⚠️ Manual | ⚠️ Manual | Compiled | Moderate (pip/conda) |

**Production Deployment Leader**: FEAGI (Docker, Kubernetes, REST API, ZMQ streams)

---

## Community & Ecosystem

| Framework | Academic Adoption | Industry Use | Publications | Community Size | Educational Resources |
|-----------|-------------------|--------------|--------------|----------------|-----------------------|
| **FEAGI** | Emerging | Robotics, AGI | Growing | Growing | Moderate |
| **Lava** | Established | Intel, research | Multiple | Established | Good (tutorials) |
| **Nengo** | ✅✅✅ Widespread | Research, education | 100+ (incl. *Science*) | Large | ✅✅✅ Extensive |
| **CARLsim** | Established | UC Irvine labs | 10+ | Medium | Moderate |
| **snnTorch** | Growing | Education, research | Multiple | Growing | ✅✅✅ Excellent (20+ tutorials) |
| **GeNN** | Established | Sussex, research | 4+ (*Nature*, etc.) | Medium | Moderate |

**Academic Leaders**:
- **Most Cited**: Nengo (100+ papers), CARLsim (100+)
- **Prestigious Publications**: Nengo (*Science*), GeNN (*Nature Comp. Sci.*)
- **Educational**: Nengo (summer school), snnTorch (20+ tutorials)

---

## Framework Strengths Summary Table

| Strength Category | Leader(s) | Runner-Up(s) |
|-------------------|-----------|--------------|
| **GPU Acceleration** | GeNN (10-100x) | CARLsim (10-50x), snnTorch |
| **Neuromorphic Hardware** | Lava (Loihi native) | Nengo (Loihi, SpiNNaker, FPGA) |
| **Real-Time Control** | FEAGI (<10ms) | Lava (Loihi: μs) |
| **Multi-Agent Systems** | FEAGI (native ZMQ) | All others (manual) |
| **Evolutionary Learning** | FEAGI (unique) | CARLsim (ECJ params only) |
| **Cognitive Modeling** | Nengo (Spaun) | - |
| **Educational Resources** | Nengo, snnTorch | - |
| **Large-Scale (100M+)** | GeNN (procedural) | CARLsim, Nengo (SpiNNaker) |
| **Production Deployment** | FEAGI (Docker/K8s) | snnTorch (Colab) |
| **Supervised Training** | snnTorch (PyTorch) | Lava (SLAYER), Nengo (NengoDL) |
| **Biological Realism** | FEAGI (development) | CARLsim (Izhikevich), GeNN (HH) |
| **Platform Independence** | FEAGI (no GPU req) | Nengo, snnTorch |
| **Power Efficiency** | Lava (1000x on Loihi) | - |
| **Visual Cortex** | CARLsim (specialized) | GeNN |
| **Parameter Tuning** | CARLsim (ECJ auto) | Manual for others |

---

## Ecosystem Maturity

| Framework | Years Active | Community Maturity | Documentation Quality | Academic Validation | Industry Adoption |
|-----------|--------------|-------------------|----------------------|---------------------|-------------------|
| **FEAGI** | ~9 years (2.0: new) | Growing | Comprehensive | Emerging | Robotics (emerging) |
| **Lava** | ~4 years | Established | Excellent | Strong | Intel ecosystem |
| **Nengo** | ~12 years | Very mature | Excellent | Very strong | Education, research |
| **CARLsim** | ~16 years | Mature | Good | Strong | Academic labs |
| **snnTorch** | ~4 years | Growing rapidly | Excellent | Growing | Education, research |
| **GeNN** | ~9 years | Mature | Excellent | Strong | Academic research |

**Maturity Leaders**:
- **Most Mature**: Nengo (12 years, widespread adoption)
- **Longest Running**: CARLsim (16 years, since 2009)
- **Best Documentation**: Nengo, snnTorch (tutorial-driven)

---

## Integration Comparison

| Framework | PyTorch | TensorFlow | ROS | Neuromorphic HW | CUDA/GPU | WebAssembly |
|-----------|---------|------------|-----|-----------------|----------|-------------|
| **FEAGI** | 📋 | 📋 | ✅ Bridge | 📋 Planned | 📋 Planned | 🚧 Planned |
| **Lava** | ⚠️ Via SLAYER | ⚠️ Via SLAYER | ⚠️ Manual | ✅ Loihi | ✅ Yes | ❌ |
| **Nengo** | ✅ pytorch-spiking | ✅ NengoDL | ⚠️ Manual | ✅ Loihi, SpiNNaker, FPGA | ✅ NengoDL | ❌ |
| **CARLsim** | ❌ | ❌ | ⚠️ Manual C++ | ❌ | ✅ Native | ❌ |
| **snnTorch** | ✅ Native | ⚠️ Via ONNX | ⚠️ Manual | 🚧 Research | ✅ Native | ❌ |
| **GeNN** | ❌ | ❌ | ⚠️ Manual | ❌ | ✅ Native | 📋 WebGPU? |

---

## Installation & Setup Comparison

| Framework | Installation Command | Prerequisites | GPU Required | Time to First Run |
|-----------|---------------------|---------------|--------------|-------------------|
| **FEAGI** | `pip install feagi` | Python (Rust pre-built) | ❌ No | <5 minutes |
| **Lava** | `pip install lava-nc` | Python | ❌ No | <5 minutes |
| **Nengo** | `pip install nengo nengo-gui` | Python | ❌ No | <5 minutes |
| **CARLsim** | Build from source (CMake) | C++, CUDA Toolkit | ⚠️ For speed | 30+ minutes |
| **snnTorch** | `pip install snntorch` | Python, PyTorch | ❌ No | <5 minutes |
| **GeNN** | `pip install pygenn` or Conda | Python, CUDA Toolkit | ⚠️ For speed | 5-15 minutes |

**Easiest Installation**: snnTorch, Lava, FEAGI, Nengo (simple pip install)

---

## Notable Achievements & Academic Impact

| Framework | Landmark Achievement | Citations | Major Publications |
|-----------|---------------------|-----------|-------------------|
| **FEAGI** | Evolutionary brain development | Growing | Architecture papers (emerging) |
| **Lava** | Loihi neuromorphic integration | Multiple | Intel publications |
| **Nengo** | **Spaun (2.5M neurons, 8 tasks)** | 100+ | ***Science* 2012** |
| **CARLsim** | GPU SNN simulation (2009) | 100+ | *Neural Networks*, *Frontiers* |
| **snnTorch** | Educational tutorial series | Growing | Training methods paper |
| **GeNN** | **100M+ neurons (procedural)** | 100+ | ***Nature Comp. Sci.* 2021** |

**Most Prestigious Publications**:
- **Nengo**: *Science* (Spaun, 2012)
- **GeNN**: *Nature Computational Science* (Procedural connectivity, 2021)
- **CARLsim**: *Neural Networks* (GPU acceleration, 2009)

---

## Specialization Areas

### Framework Specializations

| Specialization | Framework(s) | Why |
|----------------|--------------|-----|
| **Autonomous Robotics** | FEAGI | Multi-agent ZMQ, real-time control, evolutionary optimization |
| **Neuromorphic Hardware** | Lava, Nengo | Loihi (Lava), Multi-platform (Nengo: Loihi/SpiNNaker/FPGA) |
| **Cognitive Modeling** | Nengo | NEF, Spaun, Semantic Pointer Architecture |
| **Visual Cortex Research** | CARLsim | V1/V2/MT models, motion selectivity, saliency |
| **Deep Learning with SNNs** | snnTorch | PyTorch integration, surrogate gradients, tutorials |
| **Large-Scale Simulation** | GeNN | 100M+ neurons, procedural connectivity, code generation |
| **Educational Use** | Nengo, snnTorch | Summer school (Nengo), 20+ tutorials (snnTorch) |
| **GPU Performance** | GeNN, CARLsim | 10-100x (GeNN), 10-50x (CARLsim) |
| **Power Efficiency** | Lava | 1000x on Loihi neuromorphic hardware |
| **Evolutionary AI** | FEAGI | Genome evolution, neuroembryogenesis |

---

## Development Workflow Comparison

### FEAGI Workflow
```
1. Design genome (JSON)
2. Load genome → neuroembryogenesis
3. Connect agents (ZMQ)
4. Run real-time (online STDP)
5. Evolve genome (fitness-based)
```
**Time**: Slower iteration, evolutionary optimization

### Lava Workflow
```
1. Define processes (Python)
2. Connect via ports
3. Train (optional: SLAYER)
4. Deploy to backend (CPU/GPU/Loihi)
```
**Time**: Fast iteration, multiple backends

### Nengo Workflow
```
1. Define computation (Python)
2. Build network (NEF automatic weights)
3. Simulate or train (NengoDL)
4. Deploy to backend
```
**Time**: Very fast (NEF instant), or training (NengoDL)

### CARLsim Workflow
```
1. Define network (C++)
2. Configure plasticity
3. Compile (CUDA)
4. Run simulation (GPU)
5. Analyze (MATLAB/Python)
```
**Time**: Moderate iteration, GPU-accelerated

### snnTorch Workflow
```
1. Define network (PyTorch)
2. Train (backprop + GPU)
3. Evaluate
4. Deploy
```
**Time**: Fast iteration, familiar PyTorch

### GeNN Workflow
```
1. Define network (C++/PyGeNN)
2. Generate CUDA code
3. Compile
4. Run simulation (GPU)
5. Analyze (Python)
```
**Time**: Moderate iteration, optimized execution

---

## Performance Benchmarks

### Simulation Speed (Relative to Real-Time)

| Framework | CPU Mode | GPU Mode | Neuromorphic |
|-----------|----------|----------|--------------|
| **FEAGI** | 1-10x (Rust) | 📋 Planned | 📋 Planned |
| **Lava** | 1x (Python) | 5-10x | **1000x+ (Loihi)** |
| **Nengo** | 0.1-1x (NumPy) | 10-100x (NengoDL) | 100-1000x (Loihi/SpiNNaker) |
| **CARLsim** | 0.01-0.1x | **10-50x** | N/A |
| **snnTorch** | 0.1-1x | 10-100x | 🚧 Research |
| **GeNN** | 0.01-0.1x | **10-100x** | N/A |

**Speed Leaders**:
- **GPU**: GeNN (10-100x, code generation)
- **Neuromorphic**: Lava (1000x on Loihi)
- **CPU**: FEAGI (Rust core)

### Energy Efficiency

| Framework | CPU Power | GPU Power | Neuromorphic Power | Sparse Spikes |
|-----------|-----------|-----------|-------------------|---------------|
| **FEAGI** | ~50W | 📋 | 📋 | ✅ Event-driven |
| **Lava** | ~50W | ~200W | **~1W (Loihi)** | ✅ Event-driven |
| **Nengo** | ~50W | ~200W | ~1W (Loihi) | ✅ Event-driven |
| **CARLsim** | ~50W | ~200W | N/A | ✅ Event-driven |
| **snnTorch** | ~10W (inference) | ~200W (training) | 🚧 | ✅ Sparse computation |
| **GeNN** | ~50W | ~200W | N/A | ✅ Event-driven |

**Power Efficiency Leader**: Lava (1000x on Loihi neuromorphic hardware)

---

## Unique Features by Framework

### FEAGI - Only Framework With:
- ✅ **Genome-to-brain development** (neuroembryogenesis)
- ✅ **Evolutionary brain structure optimization**
- ✅ **Native multi-agent coordination** (ZMQ)
- ✅ **Built-in real-time agent SDK**
- ✅ **Biological development simulation**

### Lava - Only Framework With:
- ✅ **Native Intel Loihi integration**
- ✅ **1000x power efficiency** (neuromorphic hardware)
- ✅ **CSP process paradigm**
- ✅ **SLAYER 2.0** (advanced SNN training)

### Nengo - Only Framework With:
- ✅ **Neural Engineering Framework** (mathematical rigor)
- ✅ **Spaun cognitive model** (2.5M neurons, 8 tasks, *Science* 2012)
- ✅ **Semantic Pointer Architecture** (vector symbolic)
- ✅ **Annual summer school**

### CARLsim - Only Framework With:
- ✅ **Automated parameter tuning** (ECJ framework)
- ✅ **Visual cortex specialization** (V1/V2/MT)
- ✅ **Full DA-STDP + STP + homeostasis** suite
- ✅ **16 years** of continuous development

### snnTorch - Only Framework With:
- ✅ **Native PyTorch integration** (seamless DL workflow)
- ✅ **Surrogate gradients** (for spike backprop)
- ✅ **20+ tutorial notebooks** (educational excellence)
- ✅ **Easiest learning curve** (for PyTorch users)

### GeNN - Only Framework With:
- ✅ **Automatic CUDA code generation**
- ✅ **Procedural connectivity** (10-1000x memory reduction)
- ✅ **100M+ neuron simulations** (single GPU)
- ✅ **Proven to outperform neuromorphic HW** (2018 paper)

---

## Decision Guide: Which Framework to Choose?

### Choose FEAGI If You Need:
- ✅ Autonomous robots with real-time sensory-motor control
- ✅ Multi-agent coordination (swarms, multi-robot systems)
- ✅ Evolutionary optimization of brain structures
- ✅ Biological brain development modeling
- ✅ Production deployment (Docker, Kubernetes)
- ✅ Platform independence (no GPU/neuromorphic HW)
- ✅ Online unsupervised learning (STDP)

### Choose Lava If You Need:
- ✅ Intel Loihi deployment (neuromorphic hardware)
- ✅ Ultra-low power consumption (1000x efficiency)
- ✅ Constrained optimization problems
- ✅ Fast supervised training (SLAYER)
- ✅ Dynamic neural fields

### Choose Nengo If You Need:
- ✅ Cognitive phenomenon modeling (working memory, attention)
- ✅ Mathematical rigor (Neural Engineering Framework)
- ✅ Educational resources (teaching computational neuroscience)
- ✅ Multiple neuromorphic backends (Loihi, SpiNNaker, FPGA)
- ✅ Proven cognitive architectures (Spaun-like)

### Choose CARLsim If You Need:
- ✅ Visual cortex research (V1, V2, MT neurons)
- ✅ GPU acceleration (10-50x speedup)
- ✅ Automated parameter tuning (ECJ)
- ✅ Rich plasticity mechanisms (DA-STDP, STP, homeostasis)
- ✅ Established neuroscience research tool

### Choose snnTorch If You Need:
- ✅ Supervised SNN training (with labeled data)
- ✅ PyTorch ecosystem integration
- ✅ Fast GPU-accelerated training
- ✅ Educational tutorials (learning SNNs)
- ✅ Energy-efficient classification
- ✅ Easiest learning curve

### Choose GeNN If You Need:
- ✅ Large-scale simulation (100M+ neurons)
- ✅ GPU acceleration (10-100x speedup)
- ✅ Procedural connectivity (memory-efficient)
- ✅ Code generation (optimized CUDA)
- ✅ Performance benchmarking
- ✅ Highly-connected cortical models

---

## Hybrid Approaches & Synergies

Many projects can benefit from **combining multiple frameworks**:

### Example Hybrid Architectures:

**1. FEAGI + snnTorch**
```
snnTorch (supervised training) → Perception modules
         ↓
FEAGI (evolutionary control) → Real-time agent coordination
```
**Benefit**: Fast supervised perception + evolutionary control

**2. FEAGI + GeNN**
```
GeNN (GPU simulation) → Evolutionary brain development (fast)
         ↓
FEAGI (CPU deployment) → Production real-time agents
```
**Benefit**: Fast development + efficient deployment

**3. Nengo + FEAGI**
```
Nengo (cognitive modules) → Working memory, planning
         ↓
FEAGI (sensory-motor) → Real-time agent control
```
**Benefit**: Cognitive reasoning + embodied action

**4. Lava + FEAGI**
```
Lava (Loihi training) → Energy-efficient perception
         ↓
FEAGI (agent coordination) → Multi-agent deployment
```
**Benefit**: Ultra-low power + multi-agent systems

---

## Framework Selection Flowchart

```
START: What is your primary goal?
│
├─ Autonomous Robots? → FEAGI
│  └─ With ultra-low power? → Lava (Loihi) + FEAGI
│
├─ Cognitive Modeling? → Nengo
│  └─ With neuromorphic deployment? → Nengo (multi-backend)
│
├─ Supervised Classification? → snnTorch
│  └─ With neuromorphic deployment? → snnTorch → Lava/Nengo
│
├─ Visual Cortex Research? → CARLsim
│  └─ With parameter tuning? → CARLsim (ECJ framework)
│
├─ Large-Scale Simulation (100M+)? → GeNN
│  └─ With procedural connectivity? → GeNN (memory-efficient)
│
├─ Educational/Teaching? → snnTorch or Nengo
│  └─ Deep learning focus? → snnTorch
│  └─ Cognitive neuroscience? → Nengo
│
└─ Neuromorphic Hardware?
   ├─ Intel Loihi? → Lava (native) or Nengo
   ├─ SpiNNaker? → Nengo
   └─ FPGA? → Nengo
```

---

## Collaboration Matrix

**High Synergy Potential**:

| Framework Pair | Synergy Potential | Integration Approach |
|----------------|-------------------|---------------------|
| **FEAGI + snnTorch** | ✅✅✅ High | snnTorch perception → FEAGI control |
| **FEAGI + GeNN** | ✅✅✅ High | GeNN GPU backend for FEAGI |
| **FEAGI + Lava** | ✅✅ Good | FEAGI on Loihi via Lava |
| **FEAGI + Nengo** | ✅✅ Good | Nengo cognitive modules in FEAGI |
| **snnTorch + Lava** | ✅✅ Good | Train in snnTorch → deploy on Loihi |
| **GeNN + CARLsim** | ⚠️ Moderate | Both GPU simulators (overlap) |

---

## Framework Maturity Assessment

### Production-Ready
- **FEAGI**: ✅ Docker, Kubernetes, multi-agent systems
- **Lava**: ✅ Loihi deployment, Intel support
- **Nengo**: ✅ Multiple backends, mature ecosystem

### Research Tools
- **CARLsim**: Computational neuroscience research
- **snnTorch**: Educational and research
- **GeNN**: Large-scale simulation research

### Production Readiness Score

| Framework | Stability | Deployment | Support | Documentation | Overall |
|-----------|-----------|------------|---------|---------------|---------|
| **FEAGI** | ✅✅ | ✅✅✅ | ✅ | ✅✅ | **Production** |
| **Lava** | ✅✅✅ | ✅✅ | ✅✅✅ | ✅✅ | **Production** |
| **Nengo** | ✅✅✅ | ✅✅ | ✅✅✅ | ✅✅✅ | **Production** |
| **CARLsim** | ✅✅ | ⚠️ | ✅ | ✅✅ | **Research** |
| **snnTorch** | ✅✅ | ✅ | ✅ | ✅✅✅ | **Research/Edu** |
| **GeNN** | ✅✅ | ⚠️ | ✅✅ | ✅✅ | **Research** |

---

## Cost Analysis

### Hardware Costs

| Framework | Minimum Cost | Optimal Cost | Notes |
|-----------|--------------|--------------|-------|
| **FEAGI** | $500 (CPU only) | $1,500 (8-core, 32GB) | No GPU required |
| **Lava** | $500 (CPU dev) | $50,000+ (Loihi access) | Loihi hardware proprietary |
| **Nengo** | $500 (CPU) | $2,000 (GPU) or Loihi | Multiple options |
| **CARLsim** | $1,000 (basic GPU) | $3,000 (RTX 4090) | NVIDIA GPU required |
| **snnTorch** | $500 (CPU) | $2,000 (GPU) | PyTorch compatible |
| **GeNN** | $1,000 (basic GPU) | $5,000 (multi-GPU) | NVIDIA GPU required |

### Total Cost of Ownership (TCO)

| Framework | Software Cost | Hardware Cost | Training Cost | Support Cost | Total (3 years) |
|-----------|---------------|---------------|---------------|--------------|-----------------|
| **FEAGI** | $0 (Apache 2.0) | Low (CPU) | Low (no labeled data) | Community | **Low** |
| **Lava** | $0 (BSD-3) | High (Loihi) | Moderate | Intel | **High** |
| **Nengo** | $0 (BSD-3) | Low-Moderate | Moderate | Community/ABR | **Moderate** |
| **CARLsim** | $0 (MIT-like) | Moderate (GPU) | Low | Community | **Moderate** |
| **snnTorch** | $0 (MIT) | Low-Moderate | Moderate | Community | **Low-Moderate** |
| **GeNN** | $0 (GPL/LGPL) | Moderate (GPU) | Low | Community | **Moderate** |

**Lowest TCO**: FEAGI (no GPU, no labeled data, no hardware lock-in)

---

## Research Impact Comparison

### Citation Metrics

| Framework | Primary Paper Citations | Total Citations (est.) | h-index Impact |
|-----------|------------------------|------------------------|----------------|
| **FEAGI** | Growing | <50 | Emerging |
| **Lava** | Multiple Intel papers | 100+ | Medium |
| **Nengo** | **1000+ (Spaun)** | 500+ | **High** |
| **CARLsim** | 100+ (2009 paper) | 200+ | Medium-High |
| **snnTorch** | Growing | <100 | Emerging |
| **GeNN** | 100+ (2016 paper) | 150+ | Medium |

### Academic Prestige

| Framework | Top-Tier Publications | Research Adoption | University Use |
|-----------|----------------------|-------------------|----------------|
| **FEAGI** | Emerging | Growing | Emerging |
| **Lava** | Intel publications | High (neuromorphic) | Growing |
| **Nengo** | ***Science* 2012** | **Very High** | **50+ universities** |
| **CARLsim** | *Neural Networks* | High (neuroscience) | 50+ labs |
| **snnTorch** | Methods papers | Growing | Growing (education) |
| **GeNN** | ***Nature* 2021** | High (neuroscience) | 20+ universities |

**Academic Leaders**: Nengo (*Science*), GeNN (*Nature*)

---

## Learning Resources Comparison

| Framework | Official Tutorials | Video Content | Books/Papers | Community Forum | Learning Path |
|-----------|-------------------|---------------|--------------|-----------------|---------------|
| **FEAGI** | Moderate | ⚠️ Limited | Architecture docs | Discord | Self-guided |
| **Lava** | Good | ⚠️ Limited | Intel docs | GitHub | Tutorial-guided |
| **Nengo** | ✅✅✅ Extensive | ✅ Available | ✅ Textbook | ✅ Active forum | ✅ Summer school |
| **CARLsim** | Moderate | ❌ Limited | 10+ papers | ⚠️ Limited | Paper-guided |
| **snnTorch** | ✅✅✅ Excellent (20+) | ✅ Available | Methods papers | GitHub | ✅ Tutorial series |
| **GeNN** | Good | ⚠️ Limited | 4+ papers | GitHub | Documentation-guided |

**Educational Leaders**: Nengo (summer school, textbook), snnTorch (20+ tutorials)

---

## Framework Limitations Summary

### FEAGI Limitations
- ⚠️ No GPU acceleration yet (planned Q3-Q4 2025)
- ⚠️ Evolutionary learning slower than supervised
- ⚠️ Smaller academic community (growing)
- ⚠️ Fewer published benchmarks

### Lava Limitations
- ⚠️ Loihi hardware access limited (Intel partners)
- ⚠️ No built-in multi-agent framework
- ⚠️ Complex licensing (BSD-3 + LGPL)
- ⚠️ Hardware lock-in for best performance

### Nengo Limitations
- ⚠️ No built-in multi-agent support
- ⚠️ Not designed for real-time robotics
- ⚠️ No evolutionary optimization
- ⚠️ Python overhead (unless specialized backend)

### CARLsim Limitations
- ⚠️ Requires NVIDIA GPU for performance
- ⚠️ No macOS GPU support (CUDA-only)
- ⚠️ Build from source (complex installation)
- ⚠️ No built-in multi-agent framework

### snnTorch Limitations
- ⚠️ Requires labeled training data
- ⚠️ No evolutionary optimization
- ⚠️ Not designed for real-time control
- ⚠️ Limited continual learning support

### GeNN Limitations
- ⚠️ Requires NVIDIA GPU for performance
- ⚠️ No macOS GPU support (CUDA-only)
- ⚠️ Simulation-focused (not real-time control)
- ⚠️ No evolutionary brain development

---

## Market Position & Adoption

### Industry Adoption

| Framework | Primary Industries | Adoption Stage | Market Trajectory |
|-----------|-------------------|----------------|-------------------|
| **FEAGI** | Robotics, AGI | Early adopter | ↗️ Growing |
| **Lava** | Neuromorphic research, Intel | Established | → Stable |
| **Nengo** | Education, research | Mature | → Stable |
| **CARLsim** | Academic research | Mature | → Stable |
| **snnTorch** | Education, research | Growing | ↗️ Growing |
| **GeNN** | Academic research | Established | → Stable |

### Geographic Distribution

| Framework | Primary Regions | Notable Institutions |
|-----------|----------------|----------------------|
| **FEAGI** | North America, global | Neuraville Inc., partner universities |
| **Lava** | Global (Intel presence) | Intel, INRC members |
| **Nengo** | North America, Europe | ABR, 50+ universities worldwide |
| **CARLsim** | North America | UC Irvine, partner labs |
| **snnTorch** | Global (online) | Universities (education) |
| **GeNN** | Europe, global | University of Sussex, research labs |

---

## Future Trajectories (2025-2027)

| Framework | GPU Support | Neuromorphic HW | Embedded | Cloud-Native | Key Focus |
|-----------|-------------|-----------------|----------|--------------|-----------|
| **FEAGI** | 📋 2025 Q3-Q4 | 📋 2026 | 🚧 RTOS | ✅ Yes | Evolutionary AGI |
| **Lava** | ✅ Existing | ✅ Loihi 3? | ⚠️ Limited | ⚠️ Manual | Loihi ecosystem |
| **Nengo** | ✅ Existing | ✅ Expanding | ⚠️ Limited | ⚠️ Manual | Cognitive architectures |
| **CARLsim** | ✅ Existing | 📋 Possible | ⚠️ Limited | ⚠️ Manual | Visual cortex |
| **snnTorch** | ✅ Existing | 🚧 Research | ⚠️ Limited | ✅ Colab | Education + DL |
| **GeNN** | ✅ Existing | 📋 Possible | ⚠️ Limited | ⚠️ Manual | Large-scale sim |

---

## Conclusion: Framework Ecosystem Map

```
                    Neuromorphic Computing Landscape
                                    │
        ┌───────────────────────────┼───────────────────────────┐
        │                           │                           │
   HARDWARE                    SIMULATION                  APPLICATION
   FOCUSED                     FOCUSED                     FOCUSED
        │                           │                           │
    ┌───┴───┐               ┌───────┴───────┐           ┌───────┴────────┐
    │       │               │       │       │           │                │
   Lava   Nengo          CARLsim  GeNN  snnTorch      FEAGI          Nengo
(Loihi) (Multi-HW)      (GPU)   (GPU)  (PyTorch)   (Embodied)    (Cognitive)
    │       │               │       │       │           │                │
    │       │               │       │       │           │                │
Power   Multi-         Visual  Scale  Deep        Multi-Agent      Cognitive
Efficient Platform    Cortex (100M+) Learning    Real-Time         Tasks
```

### Framework Families:

**1. Neuromorphic Hardware-Focused**:
- **Lava**: Intel Loihi (exclusive)
- **Nengo**: Multi-platform (Loihi, SpiNNaker, FPGA)

**2. GPU Simulation-Focused**:
- **GeNN**: Code generation, 100M+ neurons
- **CARLsim**: Visual cortex, rich plasticity
- **snnTorch**: Deep learning integration

**3. Application-Focused**:
- **FEAGI**: Embodied AI, autonomous agents
- **Nengo**: Cognitive modeling, computation

---

## Key Takeaways

### 1. No Single "Best" Framework
Each framework excels in its domain:
- **Autonomous robots**: FEAGI
- **Neuromorphic hardware**: Lava, Nengo
- **Cognitive modeling**: Nengo
- **GPU simulation**: GeNN, CARLsim
- **Deep learning**: snnTorch
- **Education**: Nengo, snnTorch

### 2. Complementary Strengths
Frameworks can be combined:
- Train with snnTorch/Lava → Deploy with FEAGI
- Develop with GeNN (GPU) → Deploy with FEAGI (CPU)
- Cognitive modules (Nengo) + Agent control (FEAGI)

### 3. FEAGI's Unique Position
FEAGI is the **only framework** with:
- Genome-to-brain development (neuroembryogenesis)
- Evolutionary brain structure optimization
- Native multi-agent real-time coordination
- Production-ready agent deployment

### 4. Hardware Determines Performance
- **CPU-only**: FEAGI (Rust), Nengo (NumPy)
- **GPU acceleration**: GeNN (10-100x), CARLsim (10-50x)
- **Neuromorphic**: Lava (1000x on Loihi)

### 5. Different Learning Paradigms
- **Supervised**: snnTorch, Lava (SLAYER), Nengo (NengoDL)
- **Unsupervised**: FEAGI (STDP), CARLsim (STDP/DA-STDP), GeNN (STDP)
- **Evolutionary**: FEAGI (unique genome-level)
- **Mathematical**: Nengo (NEF - instant weight computation)

---

## Recommended Reading Path

For comprehensive details, read individual comparative analyses in order:

1. **Start with FEAGI vs snnTorch** (`COMPARATIVE_ANALYSIS_SNNTORCH.md`)
   - Easiest contrast: evolutionary vs deep learning
   - Clear paradigm differences

2. **Then FEAGI vs Nengo** (`COMPARATIVE_ANALYSIS_NENGO.md`)
   - Bottom-up vs top-down approaches
   - Embodied AI vs cognitive modeling

3. **Next FEAGI vs Lava** (`COMPARATIVE_ANALYSIS_LAVA.md`)
   - Platform independence vs neuromorphic hardware
   - Evolutionary vs hardware-optimized

4. **Then FEAGI vs GeNN** (`COMPARATIVE_ANALYSIS_GENN.md`)
   - CPU vs GPU acceleration
   - Real-time vs large-scale simulation

5. **Finally FEAGI vs CARLsim** (`COMPARATIVE_ANALYSIS_CARLSIM.md`)
   - Agent-centric vs visual cortex research
   - Evolution vs parameter tuning

---

## Quick Reference Card

### At a Glance: Choose Framework By...

**Your Background**:
- **Neuroscientist**: Nengo (NEF), CARLsim (cortex)
- **AI/ML Engineer**: snnTorch (PyTorch), Lava (DL)
- **Roboticist**: FEAGI (agents), Nengo (control)
- **Systems Programmer**: FEAGI (Rust), GeNN (C++/CUDA)
- **Educator**: snnTorch (tutorials), Nengo (summer school)

**Your Hardware**:
- **CPU only**: FEAGI, Nengo, snnTorch
- **NVIDIA GPU**: GeNN (best), CARLsim, snnTorch
- **Intel Loihi**: Lava (exclusive), Nengo
- **SpiNNaker**: Nengo
- **Embedded**: FEAGI (RTOS planned)

**Your Application**:
- **Autonomous robots**: FEAGI
- **Classification**: snnTorch, Lava
- **Cognitive tasks**: Nengo
- **Research simulation**: GeNN, CARLsim
- **Low power**: Lava (Loihi)

**Your Data Situation**:
- **No labeled data**: FEAGI, CARLsim, GeNN
- **Have labeled data**: snnTorch, Lava, Nengo
- **Want mathematical approach**: Nengo (NEF)

---

## References

### Individual Comparative Analyses
- `COMPARATIVE_ANALYSIS_LAVA.md` - FEAGI vs Intel Lava
- `COMPARATIVE_ANALYSIS_NENGO.md` - FEAGI vs Nengo
- `COMPARATIVE_ANALYSIS_CARLSIM.md` - FEAGI vs CARLsim
- `COMPARATIVE_ANALYSIS_SNNTORCH.md` - FEAGI vs snnTorch
- `COMPARATIVE_ANALYSIS_GENN.md` - FEAGI vs GeNN

### Official Websites
- **FEAGI**: https://feagi.org
- **Lava**: https://lava-nc.org
- **Nengo**: http://www.nengo.ai
- **CARLsim**: https://sites.socsci.uci.edu/~jkrichma/CARLsim/
- **snnTorch**: https://snntorch.readthedocs.io
- **GeNN**: https://genn-team.github.io

### Key Publications
- **Nengo/Spaun**: Eliasmith et al. (2012). *Science* 338(6111), 1202-1205
- **GeNN**: Knight & Nowotny (2021). *Nature Computational Science* 1, 136-142
- **CARLsim**: Nageswaran et al. (2009). *Neural Networks* 22: 791-800
- **FEAGI**: Architecture documentation (feagi.org)
- **snnTorch**: Eshraghian et al. (2021). "Training SNNs Using Lessons From Deep Learning"
- **Lava**: Intel Loihi papers and documentation

---

## Document Maintenance

**Update Schedule**:
- **Quarterly**: Performance benchmarks, new features
- **Bi-Annual**: Roadmap progress, new versions
- **Annual**: Major framework changes, new comparisons

**Last Updated**: November 1, 2025  
**Next Review**: February 1, 2026

---

**Contributors**: FEAGI Architecture Team  
**Contact**: feagi@neuraville.com  
**License**: Apache 2.0 (documentation)

